#![cfg_attr(not(feature = "std"), no_std, no_main)]

use ink::primitives::AccountId;
use sp_runtime::MultiAddress;

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

    impl DividendWallet {
        /// Constructor that initializes the `bool` value to the given `init_value`.
        #[ink(constructor, payable)]
        pub fn new() -> Self {
            Self {
                fund: Self::env().caller(),
            }
        }

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
        use super::DividendWallet;
        use crate::dividend_wallet::Environment;
        use ink::primitives::AccountId;

        #[ink::test]
        fn constructor_works() {
            let fund = AccountId::from([0x01; 32]);
            ink::env::test::set_caller::<Environment>(fund);
            let contract = DividendWallet::new();
            assert_eq!(contract.fund(), fund);
        }
    }

    // TODO: write e2e tests if you have time
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
