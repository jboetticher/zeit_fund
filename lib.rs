#![cfg_attr(not(feature = "std"), no_std, no_main)]

use ink::primitives::AccountId;
use sp_runtime::MultiAddress;

/*

Workflow:
1. Manager creates the ZeitFund with an initial funding goal.
2. Users add ZTG with fund() until the fund is complete, unlocking it for the manager.
    2a. It is recommended that managers also fund, to lock their tokens as a trust mechanism.
        Otherwise, there is nothing stopping the manager from dumping. By locking, their
        liquidity is locked until liquidation of the fund.
3. Manager can interact with markets.


Zeit Fund should:
- Give a single account the ability to use a smart contract to buy/redeem shares in ZTG
- Force the single account to have at least X amount of ZTG to do anything
- Allow users to buy shares of the smart contract
- Allow users to redeem shares of the smart contract


NOTE:
No dynamic insert of funds. There is a period where funds are added and afterwards no more.
Users cannot force liquidation.
Users that wish to exit can only resell the ERC20 token, not liquidate for the individual market positions.

*/

#[ink::contract]
mod zeit_fund {
    use ink::storage::Mapping;

    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct ZeitFund {
        /// Stores a single `bool` value on the storage.
        manager: AccountId,
        /// Total token supply.
        total_supply: Balance,
        /// Mapping from owner to number of owned token.
        balances: Mapping<AccountId, Balance>,
        /// Mapping of the token amount which an account is allowed to withdraw
        /// from another account.
        allowances: Mapping<(AccountId, AccountId), Balance>,
        /// The amount of ZTG that the fund has received already.
        funding_amount: Balance,
        /// Locks the manager's shares so that they can't be transferred.
        lock_manager_shares: bool
    }

    // region: Events & Errors

    /// Event emitted when a token transfer occurs.
    #[ink(event)]
    pub struct Transfer {
        #[ink(topic)]
        from: Option<AccountId>,
        #[ink(topic)]
        to: Option<AccountId>,
        value: Balance,
    }

    /// Event emitted when an approval occurs that `spender` is allowed to withdraw
    /// up to the amount of `value` tokens from `owner`.
    #[ink(event)]
    pub struct Approval {
        #[ink(topic)]
        owner: AccountId,
        #[ink(topic)]
        spender: AccountId,
        value: Balance,
    }

    /// The ERC-20 error types.
    #[derive(Debug, PartialEq, Eq, scale::Encode, scale::Decode)]
    #[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
    pub enum Error {
        /// Returned if not enough balance to fulfill a request is available.
        InsufficientBalance,
        /// Returned if not enough allowance to fulfill a request is available.
        InsufficientAllowance,
        /// Returned if only the manager is allowed to call the function.
        OnlyManagerAllowed,
        MustBeFunded,
        FundingTooMuch,
        ManagerSharesAreLocked,
    }

    /// The ERC-20 result type.
    pub type Result<T> = core::result::Result<T, Error>;

    // endregion

    impl ZeitFund {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor, payable)]
        pub fn new(manager: AccountId, total_shares: Balance, lock_manager_shares: bool) -> Self {
            // Give the zero address itself the total supply, to be distributed later
            let mut balances = Mapping::default();
            balances.insert(AccountId::from([0; 32]), &total_shares);

            Self {
                manager,
                total_supply: total_shares,
                balances,
                allowances: Default::default(),
                funding_amount: 0,
                lock_manager_shares
            }
        }

        // TODO: separate impl of ERC20 trait
        // region: ERC-20

        /// Returns the total token supply.
        #[ink(message)]
        pub fn total_supply(&self) -> Balance {
            self.total_supply
        }

        /// Returns the account balance for the specified `owner`.
        ///
        /// Returns `0` if the account is non-existent.
        #[ink(message)]
        pub fn balance_of(&self, owner: AccountId) -> Balance {
            self.balance_of_impl(&owner)
        }

        /// Returns the account balance for the specified `owner`.
        ///
        /// Returns `0` if the account is non-existent.
        ///
        /// # Note
        ///
        /// Prefer to call this method over `balance_of` since this
        /// works using references which are more efficient in Wasm.
        #[inline]
        fn balance_of_impl(&self, owner: &AccountId) -> Balance {
            self.balances.get(owner).unwrap_or_default()
        }

        /// Returns the amount which `spender` is still allowed to withdraw from `owner`.
        ///
        /// Returns `0` if no allowance has been set.
        #[ink(message)]
        pub fn allowance(&self, owner: AccountId, spender: AccountId) -> Balance {
            self.allowance_impl(&owner, &spender)
        }

        /// Returns the amount which `spender` is still allowed to withdraw from `owner`.
        ///
        /// Returns `0` if no allowance has been set.
        ///
        /// # Note
        ///
        /// Prefer to call this method over `allowance` since this
        /// works using references which are more efficient in Wasm.
        #[inline]
        fn allowance_impl(&self, owner: &AccountId, spender: &AccountId) -> Balance {
            self.allowances.get((owner, spender)).unwrap_or_default()
        }

        /// Transfers `value` amount of tokens from the caller's account to account `to`.
        ///
        /// On success a `Transfer` event is emitted.
        ///
        /// # Errors
        ///
        /// Returns `InsufficientBalance` error if there are not enough tokens on
        /// the caller's account balance.
        #[ink(message)]
        pub fn transfer(&mut self, to: AccountId, value: Balance) -> Result<()> {
            let from = self.env().caller();
            self.transfer_from_to(&from, &to, value)
        }

        /// Allows `spender` to withdraw from the caller's account multiple times, up to
        /// the `value` amount.
        ///
        /// If this function is called again it overwrites the current allowance with
        /// `value`.
        ///
        /// An `Approval` event is emitted.
        #[ink(message)]
        pub fn approve(&mut self, spender: AccountId, value: Balance) -> Result<()> {
            let owner = self.env().caller();
            self.allowances.insert((&owner, &spender), &value);
            self.env().emit_event(Approval {
                owner,
                spender,
                value,
            });
            Ok(())
        }

        /// Transfers `value` tokens on the behalf of `from` to the account `to`.
        ///
        /// This can be used to allow a contract to transfer tokens on ones behalf and/or
        /// to charge fees in sub-currencies, for example.
        ///
        /// On success a `Transfer` event is emitted.
        ///
        /// # Errors
        ///
        /// Returns `InsufficientAllowance` error if there are not enough tokens allowed
        /// for the caller to withdraw from `from`.
        ///
        /// Returns `InsufficientBalance` error if there are not enough tokens on
        /// the account balance of `from`.
        #[ink(message)]
        pub fn transfer_from(
            &mut self,
            from: AccountId,
            to: AccountId,
            value: Balance,
        ) -> Result<()> {
            let caller = self.env().caller();
            let allowance = self.allowance_impl(&from, &caller);
            if allowance < value {
                return Err(Error::InsufficientAllowance);
            }
            self.transfer_from_to(&from, &to, value)?;
            self.allowances
                .insert((&from, &caller), &(allowance - value));
            Ok(())
        }

        /// Transfers `value` amount of tokens from the caller's account to account `to`.
        ///
        /// On success a `Transfer` event is emitted.
        ///
        /// # Errors
        ///
        /// Returns `InsufficientBalance` error if there are not enough tokens on
        /// the caller's account balance.
        fn transfer_from_to(
            &mut self,
            from: &AccountId,
            to: &AccountId,
            value: Balance,
        ) -> Result<()> {
            let from_balance = self.balance_of_impl(from);
            if from_balance < value {
                return Err(Error::InsufficientBalance);
            }

            if from == &self.manager && self.lock_manager_shares {
                return Err(Error::ManagerSharesAreLocked);
            }

            self.balances.insert(from, &(from_balance - value));
            let to_balance = self.balance_of_impl(to);
            self.balances.insert(to, &(to_balance + value));
            self.env().emit_event(Transfer {
                from: Some(*from),
                to: Some(*to),
                value,
            });
            Ok(())
        }

        // endregion

        // region: Funding

        #[ink(message, payable)]
        pub fn fund(&mut self) -> Result<()> {
            let v = self.env().transferred_value();
            // TODO: potential DOS here
            if v + self.funding_amount > self.total_supply {
                return Err(Error::FundingTooMuch);
            }

            // Mint to user
            self.transfer_from_to(&AccountId::from([0; 32]), &self.env().caller(), v)?;
            self.funding_amount += v;

            Ok(())
        }

        #[ink(message)]
        pub fn initial_funding_amount(&self) -> u128 {
            self.funding_amount
        }

        pub fn is_funded(&self) -> bool {
            self.funding_amount == self.total_supply
        }

        #[inline]
        fn must_be_funded(&self) -> Result<()> {
            if !self.is_funded() {
                return Err(Error::MustBeFunded);
            }
            Ok(())
        }

        // endregion

        // region

        #[inline]
        fn only_manager(&self) -> Result<()> {
            if self.env().caller() != self.manager {
                return Err(Error::OnlyManagerAllowed);
            }
            Ok(())
        }

        #[ink(message)]
        pub fn manager_shares(&self) -> u128 {
            self.balance_of(self.manager)
        }

        #[ink(message)]
        pub fn manager_is_locked(&self) -> bool {
            self.lock_manager_shares
        }
    }

    /// Unit tests in Rust are normally defined within such a `#[cfg(test)]`
    /// module and test functions are marked with a `#[test]` attribute.
    /// The below code is technically just normal Rust code.
    #[cfg(test)]
    mod tests {
        // Imports all the definitions from the outer scope so we can use them here.
        // use super::*;

        // TODO: write tests if you have time

        use ink::primitives::AccountId;
        use super::ZeitFund;
        use crate::zeit_fund::{Environment, Error};

        #[ink::test]
        fn funding_works() {
            let caller = AccountId::from([0x01; 32]);
            let total_shares = 1_000_000_000_000;
            let mut contract = ZeitFund::new(caller, total_shares, true);

            assert_eq!(contract.balance_of(AccountId::from([0; 32])), total_shares);

            let half_transfer = 500_000_000_000;
            ink::env::test::set_account_balance::<ink::env::DefaultEnvironment>(
                caller, half_transfer,
            );

            // Assert transfers
            ink::env::pay_with_call!(contract.fund(), half_transfer).unwrap();
            let balance = contract.balance_of(caller);
            assert_eq!(balance, half_transfer);
            ink::env::pay_with_call!(contract.fund(), half_transfer).unwrap();
            let balance = contract.balance_of(caller);
            assert_eq!(balance, total_shares);

            // Assert that goal is reached
            assert_eq!(contract.is_funded(), true);

            // Assert failure to transfer over
            ink::env::test::set_account_balance::<ink::env::DefaultEnvironment>(
                caller, 1,
            );
            let res = ink::env::pay_with_call!(contract.fund(), 1);
            assert_eq!(res, Err(Error::FundingTooMuch));

        }
    
        #[ink::test]
        fn manager_token_lock_works() {
            let manager = AccountId::from([0x01; 32]);
            let total_shares = 1_000_000_000_000;
            let mut contract = ZeitFund::new(manager, total_shares, true);

            assert_eq!(contract.balance_of(AccountId::from([0; 32])), total_shares);
            assert_eq!(contract.manager_is_locked(), true);

            // Manager will fund with 50
            let half_transfer = 500_000_000_000;
            ink::env::test::set_account_balance::<ink::env::DefaultEnvironment>(
                manager, half_transfer,
            );
            ink::env::pay_with_call!(contract.fund(), half_transfer).unwrap();
            assert_eq!(contract.balance_of(manager), half_transfer);
            assert_eq!(contract.manager_shares(), half_transfer);

            // Assert that the manager can't transfer
            let res = contract.transfer(AccountId::from([0x08; 32]), half_transfer);
            assert_eq!(res, Err(Error::ManagerSharesAreLocked));
        }
    }
}

#[derive(scale::Encode, scale::Decode)]
enum RuntimeCall {
    /// This index can be found by investigating runtime configuration. You can check the
    /// pallet order inside `construct_runtime!` block and read the position of your
    /// pallet (0-based).
    ///
    /// https://github.com/zeitgeistpm/zeitgeist/blob/3d9bbff91219bb324f047427224ee318061a6d43/runtime/common/src/lib.rs#L254-L363
    ///
    /// [See here for more.](https://substrate.stackexchange.com/questions/778/how-to-get-pallet-index-u8-of-a-pallet-in-runtime)
    #[codec(index = 40)]
    AssetManager(AssetManagerCall),
    #[codec(index = 56)]
    Swaps(SwapsCall),
    #[codec(index = 57)]
    PredictionMarkets(PredictionMarketsCall),
}

#[derive(scale::Encode, scale::Decode)]
enum AssetManagerCall {
    // https://github.com/open-web3-stack/open-runtime-module-library/blob/22a4f7b7d1066c1a138222f4546d527d32aa4047/currencies/src/lib.rs#L129-L131C19
    #[codec(index = 0)]
    Transfer {
        dest: MultiAddress<AccountId, ()>,
        currency_id: ZeitgeistAsset,
        #[codec(compact)]
        amount: u128,
    },
}

#[derive(scale::Encode, scale::Decode)]
enum SwapsCall {
    // https://polkadot.js.org/apps/?rpc=wss%3A%2F%2Fbsr.zeitgeist.pm#/extrinsics/decode/0x380981040402286bee00b102000000000000000000000000000001000100cdbe7b00000000000000000000000000
    #[codec(index = 9)]
    SwapExactAmountIn {
        #[codec(compact)]
        pool_id: u128,
        asset_in: ZeitgeistAsset,
        #[codec(compact)]
        asset_amount_in: u128,
        asset_out: ZeitgeistAsset,
        min_asset_amount_out: Option<u128>,
        max_price: Option<u128>,
    },
    #[codec(index = 10)]
    SwapExactAmountOut {
        #[codec(compact)]
        pool_id: u128,
        asset_in: ZeitgeistAsset,
        max_asset_amount_in: Option<u128>,
        asset_out: ZeitgeistAsset,
        #[codec(compact)]
        asset_amount_out: u128,
        max_price: Option<u128>,
    },
}

#[derive(scale::Encode, scale::Decode)]
enum PredictionMarketsCall {
    #[codec(index = 5)]
    BuyCompleteSet {
        #[codec(compact)]
        market_id: u128,
        #[codec(compact)]
        amount: u128,
    },
    #[codec(index = 12)]
    RedeemShares {
        #[codec(compact)]
        market_id: u128,
    },
    #[codec(index = 15)]
    SellCompleteSet {
        #[codec(compact)]
        market_id: u128,
        #[codec(compact)]
        amount: u128,
    },
}

#[derive(scale::Encode, scale::Decode, Clone, PartialEq)]
enum ZeitgeistAsset {
    CategoricalOutcome, //(MI, CategoryIndex),
    ScalarOutcome,      //(MI, ScalarPosition),
    CombinatorialOutcome,
    PoolShare, //(SerdeWrapper<PoolId>),
    Ztg,       // default
    ForeignAsset(u32),
}
