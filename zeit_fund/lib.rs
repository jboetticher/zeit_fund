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
3. Manager can interact with markets & issue dividends of ZTG.



NOTE:
No dynamic insert of funds. There is a period where funds are added and afterwards no more.
Users cannot force liquidation.
Users that wish to exit can only resell the ERC20 token, not liquidate for the individual market positions.

NOTE:
self.env().block_number() is broken for some reason. Fortunately self.env().block_timestamp() works.
Hence, we are using timestamp instead of block_number. Be sure to change this back if the issue is
ever fixed.

TODO: check to see if env().block_number() works on substrate contracts node & make an issue

*/

#[ink::contract]
mod zeit_fund {
    use crate::{AssetManagerCall, PredictionMarketsCall, RuntimeCall, SwapsCall, ZeitgeistAsset};
    use dividend_wallet::DividendWalletRef;
    use ink::env::call::FromAccountId;
    use ink::env::Error as EnvError;
    use ink::prelude::vec::Vec;
    use ink::storage::Mapping;
    use ink::ToAccountId;

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
        lock_manager_shares: bool,
        /// The wallet that dividends are issued to so that they can no longer be used
        /// by the manager.
        dividend_wallet: DividendWalletRef,
        /// An array of dividends being issued at certain blocks.
        dividends: Vec<(Timestamp, Balance)>,
        /// The last time that a user claimed a dividend.
        last_claimed_dividend: Mapping<AccountId, Timestamp>,
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

    /// Event emitted when the manager issues a dividend.
    #[ink(event)]
    pub struct DividendIssued {
        amount: Balance,
        timestamp: Timestamp,
    }

    #[ink(event)]
    pub struct DividendClaimed {
        #[ink(topic)]
        user: AccountId,
        amount: Balance,
        timestamp: Timestamp,
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
        CallRuntimeFailed,
        DividendDistributionError,
    }

    impl From<EnvError> for Error {
        fn from(e: EnvError) -> Self {
            match e {
                EnvError::CallRuntimeFailed => Error::CallRuntimeFailed,
                _ => panic!("Unexpected error from `pallet-contracts`."),
            }
        }
    }

    /// The ERC-20 result type.
    pub type Result<T> = core::result::Result<T, Error>;

    // endregion

    impl ZeitFund {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor)]
        pub fn new(
            manager: AccountId,
            total_shares: Balance,
            lock_manager_shares: bool,
            dividend_wallet_hash: Hash,
        ) -> Self {
            // Give the zero address itself the total supply, to be distributed later
            let mut balances = Mapping::default();
            balances.insert(AccountId::from([0; 32]), &total_shares);

            // Constructs wallet
            let dividend_wallet = DividendWalletRef::new()
                .code_hash(dividend_wallet_hash)
                .endowment(0)
                .salt_bytes([0xDE, 0xAD, 0xBE, 0xEF])
                .instantiate();

            Self {
                manager,
                total_supply: total_shares,
                balances,
                allowances: Default::default(),
                funding_amount: 0,
                lock_manager_shares,
                dividend_wallet,
                dividends: Vec::new(),
                last_claimed_dividend: Default::default(),
            }
        }

        /// Constructor that takes in a dividend wallet instead of creating its own.
        ///
        /// The dividend wallet must implement the `distribute(dest: AccountId, amount: u128)`
        /// function.
        #[ink(constructor)]
        pub fn no_instantiation(
            manager: AccountId,
            total_shares: Balance,
            lock_manager_shares: bool,
            dividend_wallet: AccountId,
        ) -> Self {
            // Give the zero address itself the total supply, to be distributed later
            let mut balances = Mapping::default();
            balances.insert(AccountId::from([0; 32]), &total_shares);

            Self {
                manager,
                total_supply: total_shares,
                balances,
                allowances: Default::default(),
                funding_amount: 0,
                lock_manager_shares,
                dividend_wallet: DividendWalletRef::from_account_id(dividend_wallet),
                dividends: Vec::new(),
                last_claimed_dividend: Default::default(),
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

            // Ensure that dividend is claimed by the from & to
            // NOTE: this forces the "to" to receive the ZTG
            self.claim_dividend(from.clone())?;
            self.claim_dividend(to.clone())?;

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

        /// Allows users to send ZTG to fund the contract in return for shares.
        #[ink(message, payable)]
        pub fn fund(&mut self) -> Result<()> {
            let v = self.env().transferred_value();
            // NOTE: potential DOS here
            if v + self.funding_amount > self.total_supply {
                return Err(Error::FundingTooMuch);
            }

            // Mint to user
            self.transfer_from_to(&AccountId::from([0; 32]), &self.env().caller(), v)?;
            self.funding_amount += v;

            Ok(())
        }

        /// The initial funding amount in ZTG required for the fund to start.
        #[ink(message)]
        pub fn initial_funding_amount(&self) -> u128 {
            self.funding_amount
        }

        /// True if the contract has been completely funded, false if otherwise.
        #[ink(message)]
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

        // region: Fund Management

        /// Allows the manager to send a call into the Swaps pallet.
        #[ink(message)]
        pub fn swap_call(&mut self, call: SwapsCall) -> Result<()> {
            self.only_manager()?;
            self.must_be_funded()?;

            self.env()
                .call_runtime(&RuntimeCall::Swaps(call))
                .map_err(Into::<Error>::into)?;

            Ok(())
        }

        /// Allows the manager to send a call into the PredictionMarkets pallet.
        #[ink(message)]
        pub fn prediction_market_call(&mut self, call: PredictionMarketsCall) -> Result<()> {
            self.only_manager()?;
            self.must_be_funded()?;

            self.env()
                .call_runtime(&RuntimeCall::PredictionMarkets(call))
                .map_err(Into::<Error>::into)?;

            Ok(())
        }

        // endregion

        // region: Dividends

        /// Allows the manager to issue a dividend of a specific amount.
        #[ink(message)]
        pub fn issue_dividend(&mut self, amount: Balance) -> Result<()> {
            self.only_manager()?;
            self.must_be_funded()?;

            // Send to dividend wallet
            self.env()
                .call_runtime(&RuntimeCall::AssetManager(AssetManagerCall::Transfer {
                    dest: self.dividend_wallet.to_account_id().into(),
                    currency_id: ZeitgeistAsset::Ztg,
                    amount,
                }))
                .map_err(Into::<Error>::into)?;

            // Add to dividend list
            let timestamp = self.env().block_timestamp();
            self.dividends.push((timestamp, amount));

            // Emit dividend event
            self.env().emit_event(DividendIssued { amount, timestamp });

            Ok(())
        }

        /// Claims a dividend for the caller.
        #[ink(message)]
        pub fn claim(&mut self) -> Result<Balance> {
            self.claim_dividend(self.env().caller())
        }

        /// Claims a dividend for a specific user
        fn claim_dividend(&mut self, caller: AccountId) -> Result<Balance> {
            // Calculate amount of dividend since last claim
            let dividend = self.calc_dividend(caller);

            // Sets last claimed dividend
            let block_timestamp = self.env().block_timestamp();
            self.last_claimed_dividend.insert(caller, &block_timestamp);

            // Claim dividend from dividend wallet
            if dividend > 0 {
                let res = self.dividend_wallet.distribute(caller, dividend);
                if !res {
                    return Err(Error::DividendDistributionError);
                }

                self.env().emit_event(DividendClaimed {
                    user: caller,
                    amount: dividend,
                    timestamp: block_timestamp,
                });
            }

            Ok(dividend)
        }

        /// The dividend that a specific AccountId is currently entitled to.
        #[ink(message)]
        pub fn calc_dividend(&self, user: AccountId) -> Balance {
            let last_block = self.last_claimed_dividend.get(user).unwrap_or(0);
            let user_balance = self.balance_of(user);

            // Return 0 if user doesn't have any shares
            if user_balance == 0 {
                return 0;
            }

            // Find the index of the oldest unclaimed dividend
            // TODO: implement binary search to make more efficient
            let mut oldest_unclaimed_dividend = u32::MAX as usize;
            for i in 0..self.dividends.len() {
                if self.dividends[i].0 > last_block {
                    oldest_unclaimed_dividend = i;
                    break;
                }
            }
            if oldest_unclaimed_dividend > self.dividends.len() {
                // If the oldest unclaimed dividend is too high, then there are no other dividends
                return 0;
            }

            // Find the sum of the dividends to give out since the user last received money
            // TODO: implement binary search to make more efficient
            let mut sum = 0;
            for i in oldest_unclaimed_dividend..self.dividends.len() {
                sum += self.dividends[i].1;
            }

            // Get the % of the fund that the user owns & calculate dividend from the sum
            let buffer = 1_000_000_000_000;
            let percentage = (user_balance * buffer) / self.total_supply;
            let dividend = (sum * percentage) / buffer;

            dividend
        }

        #[ink(message)]
        pub fn last_dividend_claim(&self, user: AccountId) -> Timestamp {
            self.last_claimed_dividend.get(user).unwrap_or(0)
        }

        /// The AccountId of the dividend wallet that this fund uses.
        #[ink(message)]
        pub fn dividend_wallet(&self) -> AccountId {
            self.dividend_wallet.to_account_id()
        }

        /// The AccountId of the fund registered by the dividend wallet that this fund uses.
        /// It should be the same as this smart contract. Otherwise, dividends may be handled
        /// by a third party smart contract or address.
        #[ink(message)]
        pub fn dividend_wallet_fund(&self) -> AccountId {
            self.dividend_wallet.fund()
        }

        // endregion

        #[inline]
        fn only_manager(&self) -> Result<()> {
            if self.env().caller() != self.manager {
                return Err(Error::OnlyManagerAllowed);
            }
            Ok(())
        }

        /// The shares that the manager owns. Should be high so that they have some skin in
        /// the game!
        #[ink(message)]
        pub fn manager_shares(&self) -> u128 {
            self.balance_of(self.manager)
        }

        /// If true, the manager cannot transfer their shares (and thus cannot easily rug).
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

        use super::ZeitFund;
        use crate::zeit_fund::{Environment, Error};
        use ink::primitives::AccountId;

        /// Creates a fund without a dividend wallet (for testing purposes).
        fn create_fund_no_wallet(
            manager: AccountId,
            total_shares: u128,
            lock_manager_shares: bool,
        ) -> ZeitFund {
            ZeitFund::no_instantiation(manager, total_shares, lock_manager_shares, manager)
        }

        /// Sends a lot of ZTG/DEV to a wallet.
        fn megafund_wallet(wallet: AccountId) {
            ink::env::test::set_account_balance::<ink::env::DefaultEnvironment>(
                wallet,
                100_000_000_000_000_000,
            );
        }

        #[ink::test]
        fn funding_works() {
            let caller = AccountId::from([0x01; 32]);
            let total_shares = 1_000_000_000_000;
            let mut contract = create_fund_no_wallet(caller, total_shares, true);

            assert_eq!(contract.balance_of(AccountId::from([0; 32])), total_shares);

            let half_transfer = 500_000_000_000;
            megafund_wallet(caller);

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
            ink::env::test::set_account_balance::<ink::env::DefaultEnvironment>(caller, 1);
            let res = ink::env::pay_with_call!(contract.fund(), 1);
            assert_eq!(res, Err(Error::FundingTooMuch));
        }

        #[ink::test]
        fn manager_token_lock_works() {
            let manager = AccountId::from([0x01; 32]);
            let total_shares = 1_000_000_000_000;
            let mut contract = create_fund_no_wallet(manager, total_shares, true);

            assert_eq!(contract.balance_of(AccountId::from([0; 32])), total_shares);
            assert_eq!(contract.manager_is_locked(), true);

            // Manager will fund with 50
            let half_transfer = 500_000_000_000;
            ink::env::test::set_account_balance::<ink::env::DefaultEnvironment>(
                manager,
                half_transfer,
            );
            ink::env::pay_with_call!(contract.fund(), half_transfer).unwrap();
            assert_eq!(contract.balance_of(manager), half_transfer);
            assert_eq!(contract.manager_shares(), half_transfer);

            // Assert that the manager can't transfer
            let res = contract.transfer(AccountId::from([0x08; 32]), half_transfer);
            assert_eq!(res, Err(Error::ManagerSharesAreLocked));
        }

        #[ink::test]
        fn token_based_dividend_calculation_works() {
            let manager = AccountId::from([0x01; 32]);
            let user = AccountId::from([0x04; 32]);
            let total_shares = 100_000_000_000;
            let mut fund = create_fund_no_wallet(manager, total_shares, false);

            // Manager will fund with 1/4 of the shares
            let quarter_transfer = total_shares / 4;
            megafund_wallet(manager);
            ink::env::pay_with_call!(fund.fund(), quarter_transfer).unwrap();
            assert_eq!(fund.balance_of(manager), quarter_transfer);
            assert_eq!(fund.manager_shares(), quarter_transfer);

            // Another account will fund with 3/4 of the shares
            ink::env::test::set_caller::<Environment>(user);
            megafund_wallet(user);
            ink::env::pay_with_call!(fund.fund(), quarter_transfer * 3).unwrap();
            assert_eq!(fund.balance_of(user), quarter_transfer * 3);
            assert!(fund.is_funded());

            // NOTE:    Cannot do fund.issue_dividend() since it calls runtime. Instead,
            //          we manually add to the dividend.

            // "Issue" dividend by cheating
            let dividend_amount = total_shares / 2;
            fund.dividends.push((100_000_000, dividend_amount));

            // Claim values should be proportional to the tokens
            let manager_dividend = fund.calc_dividend(manager);
            assert_eq!(manager_dividend, dividend_amount / 4);
            let user_dividend = fund.calc_dividend(user);
            assert_eq!(user_dividend, dividend_amount / 4 * 3);

            // "Issue" second dividend by cheating
            let second_dividend_amount = total_shares / 4;
            fund.dividends.push((100_000_000, second_dividend_amount));

            // Claim values should sum up
            let manager_dividend = fund.calc_dividend(manager);
            assert_eq!(
                manager_dividend,
                (dividend_amount + second_dividend_amount) / 4
            );
            let user_dividend = fund.calc_dividend(user);
            assert_eq!(
                user_dividend,
                (dividend_amount + second_dividend_amount) / 4 * 3
            );
        }
    }
}

#[derive(scale::Encode, scale::Decode)]
pub enum RuntimeCall {
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
pub enum AssetManagerCall {
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
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub enum SwapsCall {
    #[codec(index = 1)]
    PoolExit {
        #[codec(compact)]
        pool_id: u128,
        #[codec(compact)]
        pool_amount: u128,
        min_assets_out: Vec<u128>,
    },
    #[codec(index = 5)]
    PoolJoin {
        #[codec(compact)]
        pool_id: u128,
        #[codec(compact)]
        pool_amount: u128,
        max_assets_in: Vec<u128>,
    },
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
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub enum PredictionMarketsCall {
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
#[cfg_attr(feature = "std", derive(scale_info::TypeInfo))]
pub enum ZeitgeistAsset {
    CategoricalOutcome(u128, u16),
    ScalarOutcome, //(u128, ScalarPosition),
    CombinatorialOutcome,
    PoolShare, //(SerdeWrapper<PoolId>),
    Ztg,       // default
    ForeignAsset(u32),
}

// ink::storage::traits::StorageLayout,
