// Copyright (c) The Diem Core Contributors
// Copyright (c) The Move Contributors
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::unit_arg)]
#![allow(clippy::arc_with_non_send_sync)]

use crate::language_storage::ModuleId;
use anyhow::Result;
#[cfg(any(test, feature = "fuzzing"))]
use proptest::prelude::*;
#[cfg(any(test, feature = "fuzzing"))]
use proptest_derive::Arbitrary;
use serde::{de, ser, Deserialize, Serialize};
use std::{convert::TryFrom, fmt};

/// The minimum status code for validation statuses
pub static VALIDATION_STATUS_MIN_CODE: u64 = 0;

/// The maximum status code for validation statuses
pub static VALIDATION_STATUS_MAX_CODE: u64 = 999;

/// The minimum status code for verification statuses
pub static VERIFICATION_STATUS_MIN_CODE: u64 = 1000;

/// The maximum status code for verification statuses
pub static VERIFICATION_STATUS_MAX_CODE: u64 = 1999;

/// The minimum status code for invariant violation statuses
pub static INVARIANT_VIOLATION_STATUS_MIN_CODE: u64 = 2000;

/// The maximum status code for invariant violation statuses
pub static INVARIANT_VIOLATION_STATUS_MAX_CODE: u64 = 2999;

/// The minimum status code for deserialization statuses
pub static DESERIALIZATION_STATUS_MIN_CODE: u64 = 3000;

/// The maximum status code for deserialization statuses
pub static DESERIALIZATION_STATUS_MAX_CODE: u64 = 3999;

/// The minimum status code for runtime statuses
pub static EXECUTION_STATUS_MIN_CODE: u64 = 4000;

/// The maximum status code for runtim statuses
pub static EXECUTION_STATUS_MAX_CODE: u64 = 4999;

/// A `VMStatus` is represented as either
/// - `Executed` indicating successful execution
/// - `Error` indicating an error from the VM itself
/// - `MoveAbort` indicating an `abort` ocurred inside of a Move program
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
#[cfg_attr(any(test, feature = "fuzzing"), proptest(no_params))]
pub enum VMStatus {
    /// The VM status corresponding to an EXECUTED status code
    Executed,

    /// Indicates an error from the VM, e.g. OUT_OF_GAS, INVALID_AUTH_KEY, RET_TYPE_MISMATCH_ERROR
    /// etc.
    /// The code will neither EXECUTED nor ABORTED
    Error {
        status_code: StatusCode,
        sub_status: Option<u64>,
        message: Option<String>,
    },

    /// Indicates an `abort` from inside Move code. Contains the location of the abort and the code
    MoveAbort(AbortLocation, /* code */ u64),

    /// Indicates an failure from inside Move code, where the VM could not continue exection, e.g.
    /// dividing by zero or a missing resource
    ExecutionFailure {
        status_code: StatusCode,
        sub_status: Option<u64>,
        location: AbortLocation,
        function: u16,
        code_offset: u16,
        message: Option<String>,
    },
}

/// Helper function to return `Some(s.into::<String>())` to fit in `ExecutionFailure::message`.
pub fn err_msg<S: Into<String>>(s: S) -> Option<String> {
    Some(s.into())
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
#[cfg_attr(any(test, feature = "fuzzing"), proptest(no_params))]
pub enum KeptVMStatus {
    Executed,
    OutOfGas,
    MoveAbort(AbortLocation, /* code */ u64),
    ExecutionFailure {
        location: AbortLocation,
        function: u16,
        code_offset: u16,
        message: Option<String>,
    },
    MiscellaneousError,
}

impl KeptVMStatus {
    pub fn is_success(&self) -> bool {
        matches!(self, KeptVMStatus::Executed)
    }
}

pub type DiscardedVMStatus = StatusCode;

/// An `AbortLocation` specifies where a Move program `abort` occurred, either in a function in
/// a module, or in a script
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "fuzzing"), derive(Arbitrary))]
#[cfg_attr(any(test, feature = "fuzzing"), proptest(no_params))]
pub enum AbortLocation {
    /// Indicates `abort` occurred in the specified module
    Module(ModuleId),
    /// Indicates the `abort` occurred in a script
    Script,
}

/// A status type is one of 5 different variants, along with a fallback variant in the case that we
/// don't recognize the status code.
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum StatusType {
    Validation,
    Verification,
    InvariantViolation,
    Deserialization,
    Execution,
    Unknown,
}

impl VMStatus {
    pub fn error(status_code: StatusCode, message: Option<String>) -> Self {
        Self::Error {
            status_code,
            sub_status: None,
            message,
        }
    }

    /// Return the status code for the `VMStatus`
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Executed => StatusCode::EXECUTED,
            Self::MoveAbort(_, _) => StatusCode::ABORTED,
            Self::ExecutionFailure { status_code, .. } => *status_code,
            Self::Error {
                status_code: code, ..
            } => {
                let code = *code;
                debug_assert!(code != StatusCode::EXECUTED);
                debug_assert!(code != StatusCode::ABORTED);
                code
            },
        }
    }

    pub fn sub_status(&self) -> Option<u64> {
        match self {
            Self::Error { sub_status, .. } | Self::ExecutionFailure { sub_status, .. } => {
                *sub_status
            },
            Self::Executed | Self::MoveAbort(..) => None,
        }
    }

    /// Returns the message associated with the `VMStatus`, if any.
    pub fn message(&self) -> Option<&String> {
        match self {
            Self::Error { message, .. } | Self::ExecutionFailure { message, .. } => {
                message.as_ref()
            },
            _ => None,
        }
    }

    /// Returns the Move abort code if the status is `MoveAbort`, and `None` otherwise
    pub fn move_abort_code(&self) -> Option<u64> {
        match self {
            Self::MoveAbort(_, code) => Some(*code),
            Self::Error { .. } | Self::ExecutionFailure { .. } | Self::Executed => None,
        }
    }

    /// Return the status type for this `VMStatus`. This is solely determined by the `status_code`
    pub fn status_type(&self) -> StatusType {
        self.status_code().status_type()
    }

    /// Returns `Ok` with a recorded status if it should be kept, `Err` of the error code if it
    /// should be discarded
    pub fn keep_or_discard(self) -> Result<KeptVMStatus, DiscardedVMStatus> {
        match self {
            VMStatus::Executed => Ok(KeptVMStatus::Executed),
            VMStatus::MoveAbort(location, code) => Ok(KeptVMStatus::MoveAbort(location, code)),
            VMStatus::ExecutionFailure {
                status_code: StatusCode::OUT_OF_GAS,
                ..
            }
            | VMStatus::Error {
                status_code: StatusCode::OUT_OF_GAS,
                ..
            } => Ok(KeptVMStatus::OutOfGas),

            VMStatus::ExecutionFailure {
                status_code:
                    StatusCode::EXECUTION_LIMIT_REACHED
                    | StatusCode::IO_LIMIT_REACHED
                    | StatusCode::STORAGE_LIMIT_REACHED
                    | StatusCode::TOO_MANY_DELAYED_FIELDS,
                ..
            }
            | VMStatus::Error {
                status_code:
                    StatusCode::EXECUTION_LIMIT_REACHED
                    | StatusCode::IO_LIMIT_REACHED
                    | StatusCode::STORAGE_LIMIT_REACHED
                    | StatusCode::TOO_MANY_DELAYED_FIELDS,
                ..
            } => Ok(KeptVMStatus::MiscellaneousError),

            VMStatus::ExecutionFailure {
                status_code: _status_code,
                location,
                function,
                code_offset,
                message,
                ..
            } => Ok(KeptVMStatus::ExecutionFailure {
                location,
                function,
                code_offset,
                message,
            }),
            VMStatus::Error {
                status_code: code,
                message,
                ..
            } => {
                match code.status_type() {
                    // Any unknown error should be discarded
                    StatusType::Unknown => Err(code),
                    // Any error that is a validation status (i.e. an error arising from the prologue)
                    // causes the transaction to not be included.
                    StatusType::Validation => Err(code),
                    // If the VM encountered an invalid internal state, we should discard the transaction.
                    StatusType::InvariantViolation => Err(code),
                    // A transaction that publishes code that cannot be verified will be charged.
                    StatusType::Verification => Ok(KeptVMStatus::MiscellaneousError),
                    // If we are able to decode the`SignedTransaction`, but failed to decode
                    // `SingedTransaction.raw_transaction.payload` (i.e., the transaction script),
                    // there should be a charge made to that user's account for the gas fees related
                    // to decoding, running the prologue etc.
                    StatusType::Deserialization => Ok(KeptVMStatus::MiscellaneousError),
                    // Any error encountered during the execution of the transaction will charge gas.
                    StatusType::Execution => Ok(KeptVMStatus::ExecutionFailure {
                        location: AbortLocation::Script,
                        function: 0,
                        code_offset: 0,
                        message,
                    }),
                }
            },
        }
    }
}

impl fmt::Display for StatusType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let string = match self {
            StatusType::Validation => "Validation",
            StatusType::Verification => "Verification",
            StatusType::InvariantViolation => "Invariant violation",
            StatusType::Deserialization => "Deserialization",
            StatusType::Execution => "Execution",
            StatusType::Unknown => "Unknown",
        };
        write!(f, "{}", string)
    }
}

impl fmt::Display for VMStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_type = self.status_type();
        let mut status = format!("status {:#?} of type {}", self.status_code(), status_type);

        if let VMStatus::MoveAbort(_, code) = self {
            status = format!("{} with sub status {}", status, code);
        }

        if let Some(msg) = self.message() {
            status = format!("{} with message {}", status, msg);
        }

        write!(f, "{}", status)
    }
}

impl fmt::Display for KeptVMStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "status ")?;
        match self {
            KeptVMStatus::Executed => write!(f, "EXECUTED"),
            KeptVMStatus::OutOfGas => write!(f, "OUT_OF_GAS"),
            KeptVMStatus::MiscellaneousError => write!(f, "MISCELLANEOUS_ERROR"),
            KeptVMStatus::MoveAbort(location, code) => {
                write!(f, "ABORTED with code {} in {}", code, location)
            },
            KeptVMStatus::ExecutionFailure {
                location,
                function,
                code_offset,
                message,
            } => write!(
                f,
                "EXECUTION_FAILURE at bytecode offset {} in function index {} in {} with error message {:?}",
                code_offset, function, location, message
            ),
        }
    }
}

impl fmt::Debug for VMStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VMStatus::Executed => write!(f, "EXECUTED"),
            VMStatus::Error {
                status_code,
                sub_status,
                message,
            } => f
                .debug_struct("ERROR")
                .field("status_code", status_code)
                .field("sub_status", sub_status)
                .field("message", message)
                .finish(),
            VMStatus::MoveAbort(location, code) => f
                .debug_struct("ABORTED")
                .field("code", code)
                .field("location", location)
                .finish(),
            VMStatus::ExecutionFailure {
                status_code,
                location,
                function,
                code_offset,
                message,
                sub_status,
            } => f
                .debug_struct("EXECUTION_FAILURE")
                .field("status_code", status_code)
                .field("sub_status", sub_status)
                .field("location", location)
                .field("function_definition", function)
                .field("code_offset", code_offset)
                .field("message", message)
                .finish(),
        }
    }
}

impl fmt::Debug for KeptVMStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeptVMStatus::Executed => write!(f, "EXECUTED"),
            KeptVMStatus::OutOfGas => write!(f, "OUT_OF_GAS"),
            KeptVMStatus::MoveAbort(location, code) => f
                .debug_struct("ABORTED")
                .field("code", code)
                .field("location", location)
                .finish(),
            KeptVMStatus::ExecutionFailure {
                location,
                function,
                code_offset,
                message,
            } => f
                .debug_struct("EXECUTION_FAILURE")
                .field("location", location)
                .field("function_definition", function)
                .field("code_offset", code_offset)
                .field("message", message)
                .finish(),
            KeptVMStatus::MiscellaneousError => write!(f, "MISCELLANEOUS_ERROR"),
        }
    }
}

impl fmt::Display for AbortLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AbortLocation::Script => write!(f, "Script"),
            AbortLocation::Module(id) => write!(f, "{}", id),
        }
    }
}

impl fmt::Debug for AbortLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self)
    }
}

impl std::error::Error for VMStatus {}

pub mod known_locations {
    use crate::{
        ident_str,
        identifier::IdentStr,
        language_storage::{ModuleId, CORE_CODE_ADDRESS},
        vm_status::AbortLocation,
    };
    use once_cell::sync::Lazy;

    /// The Identifier for the Account module.
    pub const CORE_ACCOUNT_MODULE_IDENTIFIER: &IdentStr = ident_str!("Account");
    /// The ModuleId for the Account module.
    pub static CORE_ACCOUNT_MODULE: Lazy<ModuleId> =
        Lazy::new(|| ModuleId::new(CORE_CODE_ADDRESS, CORE_ACCOUNT_MODULE_IDENTIFIER.to_owned()));
    /// Location for an abort in the Account module
    pub fn core_account_module_abort() -> AbortLocation {
        AbortLocation::Module(CORE_ACCOUNT_MODULE.clone())
    }

    /// The Identifier for the Account module.
    pub const DIEM_ACCOUNT_MODULE_IDENTIFIER: &IdentStr = ident_str!("DiemAccount");
    /// The ModuleId for the Account module.
    pub static DIEM_ACCOUNT_MODULE: Lazy<ModuleId> =
        Lazy::new(|| ModuleId::new(CORE_CODE_ADDRESS, DIEM_ACCOUNT_MODULE_IDENTIFIER.to_owned()));
    /// Location for an abort in the Account module
    pub fn diem_account_module_abort() -> AbortLocation {
        AbortLocation::Module(DIEM_ACCOUNT_MODULE.clone())
    }

    /// The Identifier for the Diem module.
    pub const DIEM_MODULE_IDENTIFIER: &IdentStr = ident_str!("Diem");
    /// The ModuleId for the Diem module.
    pub static DIEM_MODULE: Lazy<ModuleId> =
        Lazy::new(|| ModuleId::new(CORE_CODE_ADDRESS, DIEM_MODULE_IDENTIFIER.to_owned()));
    pub fn diem_module_abort() -> AbortLocation {
        AbortLocation::Module(DIEM_MODULE.clone())
    }

    /// The Identifier for the Designated Dealer module.
    pub const DESIGNATED_DEALER_MODULE_IDENTIFIER: &IdentStr = ident_str!("DesignatedDealer");
    /// The ModuleId for the Designated Dealer module.
    pub static DESIGNATED_DEALER_MODULE: Lazy<ModuleId> = Lazy::new(|| {
        ModuleId::new(
            CORE_CODE_ADDRESS,
            DESIGNATED_DEALER_MODULE_IDENTIFIER.to_owned(),
        )
    });
    pub fn designated_dealer_module_abort() -> AbortLocation {
        AbortLocation::Module(DESIGNATED_DEALER_MODULE.clone())
    }
}

macro_rules! derive_status_try_from_repr {
    (
        #[repr($repr_ty:ident)]
        $( #[$metas:meta] )*
        $vis:vis enum $enum_name:ident {
            $(
                $variant:ident = $value: expr
            ),*
            $( , )?
        }
    ) => {
        #[repr($repr_ty)]
        $( #[$metas] )*
        $vis enum $enum_name {
            $(
                $variant = $value
            ),*
        }

        impl std::convert::TryFrom<$repr_ty> for $enum_name {
            type Error = &'static str;
            fn try_from(value: $repr_ty) -> Result<Self, Self::Error> {
                match value {
                    $(
                        $value => Ok($enum_name::$variant),
                    )*
                    _ => Err("invalid StatusCode"),
                }
            }
        }

        #[cfg(any(test, feature = "fuzzing"))]
        const STATUS_CODE_VALUES: &'static [$repr_ty] = &[
            $($value),*
        ];
    };
}

derive_status_try_from_repr! {
#[repr(u64)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, PartialOrd, Ord)]
/// We don't derive Arbitrary on this enum because it is too large and breaks proptest. It is
/// written for a subset of these in proptest_types. We test conversion between this and protobuf
/// with a hand-written test.
pub enum StatusCode {
    // The status of a transaction as determined by the prologue.
    // Validation Errors: 0-999
    // We don't want the default value to be valid
    UNKNOWN_VALIDATION_STATUS = 0,
    // The transaction has a bad signature
    INVALID_SIGNATURE = 1,
    // Bad account authentication key
    INVALID_AUTH_KEY = 2,
    // Sequence number is too old
    SEQUENCE_NUMBER_TOO_OLD = 3,
    // Sequence number is too new
    SEQUENCE_NUMBER_TOO_NEW = 4,
    // Insufficient balance to pay for max_gas specified in the transaction.
    // Balance needs to be above max_gas_amount * gas_unit_price to proceed.
    INSUFFICIENT_BALANCE_FOR_TRANSACTION_FEE = 5,
    // The transaction has expired
    TRANSACTION_EXPIRED = 6,
    // The sending account does not exist
    SENDING_ACCOUNT_DOES_NOT_EXIST = 7,
    // This write set transaction was rejected because it did not meet the
    // requirements for one.
    REJECTED_WRITE_SET = 8,
    // This write set transaction cannot be applied to the current state.
    INVALID_WRITE_SET = 9,
    // Length of program field in raw transaction exceeded max length
    EXCEEDED_MAX_TRANSACTION_SIZE = 10,
    // This script is not in our allowlist of scripts.
    UNKNOWN_SCRIPT = 11,
    // Transaction is trying to publish a new module.
    UNKNOWN_MODULE = 12,
    // Max gas units submitted with transaction exceeds max gas units bound
    // in VM
    MAX_GAS_UNITS_EXCEEDS_MAX_GAS_UNITS_BOUND = 13,
    // Max gas units submitted with transaction not enough to cover the
    // intrinsic cost of the transaction.
    MAX_GAS_UNITS_BELOW_MIN_TRANSACTION_GAS_UNITS = 14,
    // Gas unit price submitted with transaction is below minimum gas price
    // set in the VM.
    GAS_UNIT_PRICE_BELOW_MIN_BOUND = 15,
    // Gas unit price submitted with the transaction is above the maximum
    // gas price set in the VM.
    GAS_UNIT_PRICE_ABOVE_MAX_BOUND = 16,
    // Gas specifier submitted is either malformed (not a valid identifier),
    // or does not refer to an accepted gas specifier
    INVALID_GAS_SPECIFIER = 17,
    // The sending account is frozen
    SENDING_ACCOUNT_FROZEN = 18,
    // Unable to deserialize the account blob
    UNABLE_TO_DESERIALIZE_ACCOUNT = 19,
    // The currency info was unable to be found
    CURRENCY_INFO_DOES_NOT_EXIST = 20,
    // The account sender doesn't have permissions to publish modules
    INVALID_MODULE_PUBLISHER = 21,
    // The sending account has no role
    NO_ACCOUNT_ROLE = 22,
    // The transaction's chain_id does not match the one published on-chain
    BAD_CHAIN_ID = 23,
    // The sequence number is too large and would overflow if the transaction were executed
    SEQUENCE_NUMBER_TOO_BIG = 24,
    // The gas currency is not registered as a TransactionFee currency
    BAD_TRANSACTION_FEE_CURRENCY = 25,
    // Discards a transaction, because newly added code path hasn't yet been enabled,
    // and transactions were discarded before the feature was introduced.
    // To be used for example when new variants in the transaction - like new payload or authenticator
    // types are introduced, that wouldn't be deserialized successfully by the previous binary.
    //
    // When the feature is "double" gated, i.e. new bytecode version introduces new things,
    // but we don't want all the be enabled at the same time, such that it is safe to abort,
    // use the verification error code FEATURE_NOT_ENABLED instead.
    FEATURE_UNDER_GATING = 26,
    // The number of secondary signer addresses is different from the number of secondary
    // public keys provided.
    SECONDARY_KEYS_ADDRESSES_COUNT_MISMATCH = 27,
    // There are duplicates among signers, including the sender and all the secondary signers
    SIGNERS_CONTAIN_DUPLICATES = 28,
    // The sequence nonce in the transaction is invalid (too new, too old, or already used).
    SEQUENCE_NONCE_INVALID = 29,
    // There was an error when accessing chain-specific account information
    CHAIN_ACCOUNT_INFO_DOES_NOT_EXIST = 30,
    // Multisig account specific error codes
    ACCOUNT_NOT_MULTISIG = 31,
    NOT_MULTISIG_OWNER = 32,
    MULTISIG_TRANSACTION_NOT_FOUND = 33,
    MULTISIG_TRANSACTION_INSUFFICIENT_APPROVALS = 34,
    MULTISIG_TRANSACTION_PAYLOAD_DOES_NOT_MATCH_HASH = 35,
    GAS_PAYER_ACCOUNT_MISSING = 36,
    INSUFFICIENT_BALANCE_FOR_REQUIRED_DEPOSIT = 37,
    GAS_PARAMS_MISSING = 38,
    REQUIRED_DEPOSIT_INCONSISTENT_WITH_TXN_MAX_GAS = 39,
    MULTISIG_TRANSACTION_PAYLOAD_DOES_NOT_MATCH = 40,
    ACCOUNT_AUTHENTICATION_GAS_LIMIT_EXCEEDED = 41,
    NONCE_ALREADY_USED = 42,
    EMPTY_PAYLOAD_PROVIDED = 43,
    TRANSACTION_EXPIRATION_TOO_FAR_IN_FUTURE = 44,

    // Reserved error code for future use
    RESERVED_VALIDATION_ERROR_10 = 45,
    RESERVED_VALIDATION_ERROR_11 = 46,
    RESERVED_VALIDATION_ERROR_12 = 47,
    RESERVED_VALIDATION_ERROR_13 = 48,
    RESERVED_VALIDATION_ERROR_14 = 49,
    RESERVED_VALIDATION_ERROR_15 = 50,


    // When a code module/script is published it is verified. These are the
    // possible errors that can arise from the verification process.
    // Verification Errors: 1000-1999
    UNKNOWN_VERIFICATION_ERROR = 1000,
    INDEX_OUT_OF_BOUNDS = 1001,
    INVALID_SIGNATURE_TOKEN = 1003,
    RECURSIVE_STRUCT_DEFINITION = 1005,
    FIELD_MISSING_TYPE_ABILITY = 1006,
    INVALID_FALL_THROUGH = 1007,
    NEGATIVE_STACK_SIZE_WITHIN_BLOCK = 1009,
    INVALID_MAIN_FUNCTION_SIGNATURE = 1011,
    DUPLICATE_ELEMENT = 1012,
    INVALID_MODULE_HANDLE = 1013,
    UNIMPLEMENTED_HANDLE = 1014,
    LOOKUP_FAILED = 1017,
    TYPE_MISMATCH = 1020,
    MISSING_DEPENDENCY = 1021,
    POP_WITHOUT_DROP_ABILITY = 1023,
    BR_TYPE_MISMATCH_ERROR = 1025,
    ABORT_TYPE_MISMATCH_ERROR = 1026,
    STLOC_TYPE_MISMATCH_ERROR = 1027,
    STLOC_UNSAFE_TO_DESTROY_ERROR = 1028,
    UNSAFE_RET_LOCAL_OR_RESOURCE_STILL_BORROWED = 1029,
    RET_TYPE_MISMATCH_ERROR = 1030,
    RET_BORROWED_MUTABLE_REFERENCE_ERROR = 1031,
    FREEZEREF_TYPE_MISMATCH_ERROR = 1032,
    FREEZEREF_EXISTS_MUTABLE_BORROW_ERROR = 1033,
    BORROWFIELD_TYPE_MISMATCH_ERROR = 1034,
    BORROWFIELD_BAD_FIELD_ERROR = 1035,
    BORROWFIELD_EXISTS_MUTABLE_BORROW_ERROR = 1036,
    COPYLOC_UNAVAILABLE_ERROR = 1037,
    COPYLOC_WITHOUT_COPY_ABILITY = 1038,
    COPYLOC_EXISTS_BORROW_ERROR = 1039,
    MOVELOC_UNAVAILABLE_ERROR = 1040,
    MOVELOC_EXISTS_BORROW_ERROR = 1041,
    BORROWLOC_REFERENCE_ERROR = 1042,
    BORROWLOC_UNAVAILABLE_ERROR = 1043,
    BORROWLOC_EXISTS_BORROW_ERROR = 1044,
    CALL_TYPE_MISMATCH_ERROR = 1045,
    CALL_BORROWED_MUTABLE_REFERENCE_ERROR = 1046,
    PACK_TYPE_MISMATCH_ERROR = 1047,
    UNPACK_TYPE_MISMATCH_ERROR = 1048,
    READREF_TYPE_MISMATCH_ERROR = 1049,
    READREF_WITHOUT_COPY_ABILITY = 1050,
    READREF_EXISTS_MUTABLE_BORROW_ERROR = 1051,
    WRITEREF_TYPE_MISMATCH_ERROR = 1052,
    WRITEREF_WITHOUT_DROP_ABILITY = 1053,
    WRITEREF_EXISTS_BORROW_ERROR = 1054,
    WRITEREF_NO_MUTABLE_REFERENCE_ERROR = 1055,
    INTEGER_OP_TYPE_MISMATCH_ERROR = 1056,
    BOOLEAN_OP_TYPE_MISMATCH_ERROR = 1057,
    EQUALITY_OP_TYPE_MISMATCH_ERROR = 1058,
    EXISTS_WITHOUT_KEY_ABILITY_OR_BAD_ARGUMENT = 1059,
    BORROWGLOBAL_TYPE_MISMATCH_ERROR = 1060,
    BORROWGLOBAL_WITHOUT_KEY_ABILITY= 1061,
    MOVEFROM_TYPE_MISMATCH_ERROR = 1062,
    MOVEFROM_WITHOUT_KEY_ABILITY = 1063,
    MOVETO_TYPE_MISMATCH_ERROR = 1064,
    MOVETO_WITHOUT_KEY_ABILITY= 1065,
    // The self address of a module the transaction is publishing is not the sender address
    MODULE_ADDRESS_DOES_NOT_MATCH_SENDER = 1067,
    // The module does not have any module handles. Each module or script must have at least one
    // module handle.
    NO_MODULE_HANDLES = 1068,
    POSITIVE_STACK_SIZE_AT_BLOCK_END = 1069,
    MISSING_ACQUIRES_ANNOTATION = 1070,
    EXTRANEOUS_ACQUIRES_ANNOTATION = 1071,
    DUPLICATE_ACQUIRES_ANNOTATION = 1072,
    INVALID_ACQUIRES_ANNOTATION = 1073,
    GLOBAL_REFERENCE_ERROR = 1074,
    CONSTRAINT_NOT_SATISFIED = 1075,
    NUMBER_OF_TYPE_ARGUMENTS_MISMATCH = 1076,
    LOOP_IN_INSTANTIATION_GRAPH = 1077,
    // Reported when a struct has zero fields
    ZERO_SIZED_STRUCT = 1080,
    LINKER_ERROR = 1081,
    INVALID_CONSTANT_TYPE = 1082,
    MALFORMED_CONSTANT_DATA = 1083,
    EMPTY_CODE_UNIT = 1084,
    INVALID_LOOP_SPLIT = 1085,
    INVALID_LOOP_BREAK = 1086,
    INVALID_LOOP_CONTINUE = 1087,
    UNSAFE_RET_UNUSED_VALUES_WITHOUT_DROP = 1088,
    TOO_MANY_LOCALS = 1089,
    GENERIC_MEMBER_OPCODE_MISMATCH = 1090,
    FUNCTION_RESOLUTION_FAILURE = 1091,
    INVALID_OPERATION_IN_SCRIPT = 1094,
    // The sender is trying to publish two modules with the same name in one transaction
    DUPLICATE_MODULE_NAME = 1095,
    // The sender is trying to publish a module that breaks the compatibility checks
    BACKWARD_INCOMPATIBLE_MODULE_UPDATE = 1096,
    // The updated module introduces a cyclic dependency (i.e., A uses B and B also uses A)
    CYCLIC_MODULE_DEPENDENCY = 1097,
    NUMBER_OF_ARGUMENTS_MISMATCH = 1098,
    INVALID_PARAM_TYPE_FOR_DESERIALIZATION = 1099,
    FAILED_TO_DESERIALIZE_ARGUMENT = 1100,
    NUMBER_OF_SIGNER_ARGUMENTS_MISMATCH = 1101,
    CALLED_SCRIPT_VISIBLE_FROM_NON_SCRIPT_VISIBLE = 1102,
    EXECUTE_ENTRY_FUNCTION_CALLED_ON_NON_ENTRY_FUNCTION = 1103,
    // Cannot mark the module itself as a friend
    INVALID_FRIEND_DECL_WITH_SELF = 1104,
    // Cannot declare modules outside of account address as friends
    INVALID_FRIEND_DECL_WITH_MODULES_OUTSIDE_ACCOUNT_ADDRESS = 1105,
    // Cannot declare modules that this module depends on as friends
    INVALID_FRIEND_DECL_WITH_MODULES_IN_DEPENDENCIES = 1106,
    // The updated module introduces a cyclic friendship (i.e., A friends B and B also friends A)
    CYCLIC_MODULE_FRIENDSHIP = 1107,
    // A phantom type parameter was used in a non-phantom position.
    INVALID_PHANTOM_TYPE_PARAM_POSITION = 1108,
    VEC_UPDATE_EXISTS_MUTABLE_BORROW_ERROR = 1109,
    VEC_BORROW_ELEMENT_EXISTS_MUTABLE_BORROW_ERROR = 1110,
    // Loops are too deeply nested.
    LOOP_MAX_DEPTH_REACHED = 1111,
    TOO_MANY_TYPE_PARAMETERS = 1112,
    TOO_MANY_PARAMETERS = 1113,
    TOO_MANY_BASIC_BLOCKS = 1114,
    VALUE_STACK_OVERFLOW = 1115,
    TOO_MANY_TYPE_NODES = 1116,
    VALUE_STACK_PUSH_OVERFLOW = 1117,
    MAX_DEPENDENCY_DEPTH_REACHED = 1118,
    MAX_FUNCTION_DEFINITIONS_REACHED = 1119,
    MAX_STRUCT_DEFINITIONS_REACHED = 1120,
    MAX_FIELD_DEFINITIONS_REACHED = 1121,
    TOO_MANY_BACK_EDGES = 1122,
    EVENT_METADATA_VALIDATION_ERROR = 1123,
    DEPENDENCY_LIMIT_REACHED = 1124,
    // This error indicates that unstable bytecode generated by the compiler cannot be published to mainnet
    UNSTABLE_BYTECODE_REJECTED = 1125,
    PROGRAM_TOO_COMPLEX = 1126,
    USER_DEFINED_NATIVE_NOT_ALLOWED = 1127,
    // Bound on the number of struct variants per struct exceeded.
    MAX_STRUCT_VARIANTS_REACHED = 1128,
    // A variant test has wrong argument type
    TEST_VARIANT_TYPE_MISMATCH_ERROR = 1129,
    // A variant list is empty
    ZERO_VARIANTS_ERROR = 1130,
    // A feature is not enabled, and transaction will abort and be committed on chain.
    // Use only when there is no backward incompatibility concern - as it is
    // double-gated by an additional flag, i.e. new bytecode version introduces new things,
    // but we don't want all the be enabled at the same time, such that it is safe to abort.
    //
    // If we are introducing code, that previous binary would discard such a transaction,
    // you need to use FEATURE_UNDER_GATING flag instead.
    FEATURE_NOT_ENABLED = 1131,
    // Closure mask invalid
    INVALID_CLOSURE_MASK = 1132,
    // Closure eval type is not a function
    CLOSURE_CALL_REQUIRES_FUNCTION = 1133,
    // Returned if init_module function is not valid during code publishing.
    INVALID_INIT_MODULE = 1134,

    // Reserved error code for future use
    RESERVED_VERIFICATION_ERROR_1 = 1135,
    RESERVED_VERIFICATION_ERROR_2 = 1136,
    RESERVED_VERIFICATION_ERROR_3 = 1137,
    RESERVED_VERIFICATION_ERROR_4 = 1138,

    // These are errors that the VM might raise if a violation of internal
    // invariants takes place.
    // Invariant Violation Errors: 2000-2999
    UNKNOWN_INVARIANT_VIOLATION_ERROR = 2000,
    EMPTY_VALUE_STACK = 2003,
    PC_OVERFLOW = 2005,
    VERIFICATION_ERROR = 2006,
    STORAGE_ERROR = 2008,
    INTERNAL_TYPE_ERROR = 2009,
    EVENT_KEY_MISMATCH = 2010,
    UNREACHABLE = 2011,
    VM_STARTUP_FAILURE = 2012,
    UNEXPECTED_ERROR_FROM_KNOWN_MOVE_FUNCTION = 2015,
    VERIFIER_INVARIANT_VIOLATION = 2016,
    UNEXPECTED_VERIFIER_ERROR = 2017,
    UNEXPECTED_DESERIALIZATION_ERROR = 2018,
    FAILED_TO_SERIALIZE_WRITE_SET_CHANGES = 2019,
    FAILED_TO_DESERIALIZE_RESOURCE = 2020,
    // Failed to resolve type due to linking being broken after verification
    TYPE_RESOLUTION_FAILURE = 2021,
    DUPLICATE_NATIVE_FUNCTION = 2022,
    // code invariant error while handling delayed materialization, should never happen,
    // always indicates a code bug. Delayed materialization includes handling of
    // Resource Groups and Delayed Fields. Unlike regular CODE_INVARIANT_ERROR, this
    // is a signal to BlockSTM, which it might do something about (i.e. fallback to
    // sequential execution).
    // Note: This status is created both from third_party (move) and block executor
    // (aptos-move in the adapter). In the later case, it can now also represent more
    // general invariant violations beyond delayed fields, due to the convenience of
    // handling such issues with asserts (e.g. by falling back to sequential execution).
    // TODO: can be audited and broken down into specific types, once implementation
    // is also not duplicated.
    DELAYED_FIELD_OR_BLOCKSTM_CODE_INVARIANT_ERROR = 2023,
    // Speculative error means that there was an issue because of speculative
    // reads provided to the transaction, and the transaction needs to
    // be re-executed.
    // Should never be committed on chain
    SPECULATIVE_EXECUTION_ABORT_ERROR = 2024,
    ACCESS_CONTROL_INVARIANT_VIOLATION = 2025,

    // Reserved error code for future use
    RESERVED_INVARIANT_VIOLATION_ERROR_1 = 2026,
    RESERVED_INVARIANT_VIOLATION_ERROR_2 = 2027,
    RESERVED_INVARIANT_VIOLATION_ERROR_3 = 2028,
    RESERVED_INVARIANT_VIOLATION_ERROR_4 = 2039,
    RESERVED_INVARIANT_VIOLATION_ERROR_5 = 2040,

    // Errors that can arise from binary decoding (deserialization)
    // Deserialization Errors: 3000-3999
    UNKNOWN_BINARY_ERROR = 3000,
    MALFORMED = 3001,
    BAD_MAGIC = 3002,
    UNKNOWN_VERSION = 3003,
    UNKNOWN_TABLE_TYPE = 3004,
    UNKNOWN_SIGNATURE_TYPE = 3005,
    UNKNOWN_SERIALIZED_TYPE = 3006,
    UNKNOWN_OPCODE = 3007,
    BAD_HEADER_TABLE = 3008,
    UNEXPECTED_SIGNATURE_TYPE = 3009,
    DUPLICATE_TABLE = 3010,
    UNKNOWN_ABILITY = 3013,
    UNKNOWN_NATIVE_STRUCT_FLAG = 3014,
    BAD_U16 = 3017,
    BAD_U32 = 3018,
    BAD_U64 = 3019,
    BAD_U128 = 3020,
    BAD_U256 = 3021,
    VALUE_SERIALIZATION_ERROR = 3022,
    VALUE_DESERIALIZATION_ERROR = 3023,
    CODE_DESERIALIZATION_ERROR = 3024,
    INVALID_FLAG_BITS = 3025,
    // Reserved error code for future use
    RESERVED_DESERIALIZAION_ERROR_1 = 3026,
    RESERVED_DESERIALIZAION_ERROR_2 = 3027,
    RESERVED_DESERIALIZAION_ERROR_3 = 3028,
    RESERVED_DESERIALIZAION_ERROR_4 = 3029,
    RESERVED_DESERIALIZAION_ERROR_5 = 3030,

    // Errors that can arise at runtime
    // Runtime Errors: 4000-4999
    UNKNOWN_RUNTIME_STATUS = 4000,
    EXECUTED = 4001,
    OUT_OF_GAS = 4002,
    // We tried to access a resource that does not exist under the account.
    RESOURCE_DOES_NOT_EXIST = 4003,
    // We tried to create a resource under an account where that resource
    // already exists.
    RESOURCE_ALREADY_EXISTS = 4004,
    MISSING_DATA = 4008,
    DATA_FORMAT_ERROR = 4009,
    ABORTED = 4016,
    ARITHMETIC_ERROR = 4017,
    VECTOR_OPERATION_ERROR = 4018,
    EXECUTION_STACK_OVERFLOW = 4020,
    CALL_STACK_OVERFLOW = 4021,
    VM_MAX_TYPE_DEPTH_REACHED = 4024,
    VM_MAX_VALUE_DEPTH_REACHED = 4025,
    VM_EXTENSION_ERROR = 4026,
    STORAGE_WRITE_LIMIT_REACHED = 4027,
    MEMORY_LIMIT_EXCEEDED = 4028,
    VM_MAX_TYPE_NODES_REACHED = 4029,
    EXECUTION_LIMIT_REACHED = 4030,
    IO_LIMIT_REACHED = 4031,
    STORAGE_LIMIT_REACHED = 4032,
    TYPE_TAG_LIMIT_EXCEEDED = 4033,
    // A resource was accessed in a way which is not permitted by the active access control
    // specifier.
    ACCESS_DENIED = 4034,
    // The stack of access control specifier has overflowed.
    ACCESS_STACK_LIMIT_EXCEEDED = 4035,
    // We tried to create resource with more than currently allowed number of DelayedFields
    TOO_MANY_DELAYED_FIELDS = 4036,
    // Dynamic function call errors.
    RUNTIME_DISPATCH_ERROR = 4037,
    // Struct variant not matching. This error appears on an attempt to unpack or borrow a
    // field from a value which is not of the expected variant.
    STRUCT_VARIANT_MISMATCH = 4038,
    // An unimplemented functionality in the VM.
    UNIMPLEMENTED_FUNCTIONALITY = 4039,
    // Modules are cyclic (module A uses module B which uses module A). Detected at runtime in case
    // module loading is performed lazily.
    RUNTIME_CYCLIC_MODULE_DEPENDENCY = 4040,

    // Reserved error code for future use. Always keep this buffer of well-defined new codes.
    RESERVED_RUNTIME_ERROR_1 = 4041,
    RESERVED_RUNTIME_ERROR_2 = 4042,
    RESERVED_RUNTIME_ERROR_3 = 4043,

    // A reserved status to represent an unknown vm status.
    // this is u64::MAX, but we can't pattern match on that, so put the hardcoded value in
    UNKNOWN_STATUS = 18446744073709551615,
}
}

impl StatusCode {
    /// Return the status type for this status code
    pub fn status_type(self) -> StatusType {
        let major_status_number: u64 = self.into();
        if major_status_number >= VALIDATION_STATUS_MIN_CODE
            && major_status_number <= VALIDATION_STATUS_MAX_CODE
        {
            return StatusType::Validation;
        }

        if major_status_number >= VERIFICATION_STATUS_MIN_CODE
            && major_status_number <= VERIFICATION_STATUS_MAX_CODE
        {
            return StatusType::Verification;
        }

        if major_status_number >= INVARIANT_VIOLATION_STATUS_MIN_CODE
            && major_status_number <= INVARIANT_VIOLATION_STATUS_MAX_CODE
        {
            return StatusType::InvariantViolation;
        }

        if major_status_number >= DESERIALIZATION_STATUS_MIN_CODE
            && major_status_number <= DESERIALIZATION_STATUS_MAX_CODE
        {
            return StatusType::Deserialization;
        }

        if major_status_number >= EXECUTION_STATUS_MIN_CODE
            && major_status_number <= EXECUTION_STATUS_MAX_CODE
        {
            return StatusType::Execution;
        }

        StatusType::Unknown
    }
}

// TODO(#1307)
impl ser::Serialize for StatusCode {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        serializer.serialize_u64((*self).into())
    }
}

impl<'de> de::Deserialize<'de> for StatusCode {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct StatusCodeVisitor;
        impl de::Visitor<'_> for StatusCodeVisitor {
            type Value = StatusCode;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("StatusCode as u64")
            }

            fn visit_u64<E>(self, v: u64) -> std::result::Result<StatusCode, E>
            where
                E: de::Error,
            {
                Ok(StatusCode::try_from(v).unwrap_or(StatusCode::UNKNOWN_STATUS))
            }
        }

        deserializer.deserialize_u64(StatusCodeVisitor)
    }
}

impl From<StatusCode> for u64 {
    fn from(status: StatusCode) -> u64 {
        status as u64
    }
}

/// The `Arbitrary` impl only generates validation statuses since the full enum is too large.
#[cfg(any(test, feature = "fuzzing"))]
impl Arbitrary for StatusCode {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_args: ()) -> Self::Strategy {
        (any::<usize>())
            .prop_map(|index| {
                let status_code_value = STATUS_CODE_VALUES[index % STATUS_CODE_VALUES.len()];
                StatusCode::try_from(status_code_value).unwrap()
            })
            .boxed()
    }
}

#[test]
fn test_status_codes() {
    use std::collections::HashSet;
    // Make sure that within the 0-EXECUTION_STATUS_MAX_CODE that all of the status codes succeed
    // when they should, and fail when they should.
    for possible_major_status_code in 0..=EXECUTION_STATUS_MAX_CODE {
        if STATUS_CODE_VALUES.contains(&possible_major_status_code) {
            let status = StatusCode::try_from(possible_major_status_code);
            assert!(status.is_ok());
            let to_major_status_code = u64::from(status.unwrap());
            assert_eq!(possible_major_status_code, to_major_status_code);
        } else {
            assert!(StatusCode::try_from(possible_major_status_code).is_err())
        }
    }

    let mut seen_statuses = HashSet::new();
    let mut seen_codes = HashSet::new();
    // Now make sure that all of the error codes (including any that may be out-of-range) succeed.
    // Make sure there aren't any duplicate mappings
    for major_status_code in STATUS_CODE_VALUES.iter() {
        assert!(
            !seen_codes.contains(major_status_code),
            "Duplicate major_status_code found"
        );
        seen_codes.insert(*major_status_code);
        let status = StatusCode::try_from(*major_status_code);
        assert!(status.is_ok());
        let unwrapped_status = status.unwrap();
        assert!(
            !seen_statuses.contains(&unwrapped_status),
            "Found duplicate u64 -> Status mapping"
        );
        seen_statuses.insert(unwrapped_status);
        let to_major_status_code = u64::from(unwrapped_status);
        assert_eq!(*major_status_code, to_major_status_code);
    }
}

pub mod sub_status {
    // Native Function Error sub-codes
    pub const NFE_VECTOR_ERROR_BASE: u64 = 0;
    // Failure in BCS deserialization
    pub const NFE_BCS_SERIALIZATION_FAILURE: u64 = 0x1C5;

    pub mod unknown_invariant_violation {
        // Paranoid Type checking returns an error
        pub const EPARANOID_FAILURE: u64 = 0x1;

        // Reference safety checks failure
        pub const EREFERENCE_COUNTING_FAILURE: u64 = 0x2;
    }

    pub mod type_resolution_failure {
        // User provided typetag failed to load.
        pub const EUSER_TYPE_LOADING_FAILURE: u64 = 0x1;
    }
}
