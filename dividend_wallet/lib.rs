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

NOTE:
No dynamic insert of funds. There is a period where funds are added and afterwards no more.
Users cannot force liquidation.
Users that wish to exit can only resell the ERC20 token, not liquidate for the individual market positions.

*/

// Export DividendWallet so that it can be used in zeit_fund
pub use self::dividend_wallet::DividendWalletRef;

#[ink::contract]
mod dividend_wallet {
    // use core::fmt::{Debug, Formatter};

    use crate::{AssetManagerCall, RuntimeCall};

    /// Defines the storage of your contract.
    /// Add new fields to the below struct in order
    /// to add new static storage fields to your contract.
    #[ink(storage)]
    pub struct DividendWallet {
        /// The fund that controls this wallet.
        fund: AccountId,
    }

    // impl Debug for DividendWallet {
    //     fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {

    //     }
    // }

    impl DividendWallet {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor, payable)]
        pub fn new() -> Self {
            Self {
                fund: Self::env().caller(),
            }
        }

        // #[ink(message)]
        // pub fn claim_wallet(&mut self) -> bool {
        //     if self.fund != AccountId::from([0; 32]) {
        //         self.fund = self.env().caller();
        //         return true;
        //     }

        //     false
        // }

        #[ink(message)]
        pub fn fund(&self) -> AccountId {
            self.fund
        }

        #[ink(message)]
        pub fn distribute(&mut self, dest: AccountId, amount: u128) -> bool {
            if self.env().caller() != self.fund {
                ink::env::debug_println!("Caller of DividendWallet was not its fund!");
                return false;
            }

            let res =
                self.env()
                    .call_runtime(&RuntimeCall::AssetManager(AssetManagerCall::Transfer {
                        dest: dest.into(),
                        currency_id: crate::ZeitgeistAsset::Ztg,
                        amount,
                    }));

            !res.is_err()
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

        use super::DividendWallet;
        use crate::dividend_wallet::Environment;
        use ink::primitives::AccountId;
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
