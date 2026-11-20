use anchor_lang::prelude::*;

// Error texts end up in the program code and in transaction logs: short, neutral, in English (audit No. 8, L-04).
// The order and the names must not change: error numbers are derived from them (6000 + ordinal) and the site relies on them.
#[error_code]
pub enum FdError {
    #[msg("Pools are not funded yet")]
    NotFunded,
    #[msg("Pools are already funded")]
    AlreadyFunded,
    #[msg("Vault holds less than the launch parameters require")]
    VaultUnderfunded,
    #[msg("The game has not started yet")]
    NotStarted,
    #[msg("Account is already registered")]
    AlreadyRegistered,
    #[msg("Registration required")]
    NotRegistered,
    #[msg("Insufficient game balance")]
    InsufficientBalance,
    #[msg("No free spins left this week")]
    NoFreeSpins,
    #[msg("Activity fund is empty")]
    ActivityFundEmpty,
    #[msg("Round is too large")]
    BatchTooLarge,
    #[msg("Invalid round size")]
    BadCount,
    #[msg("Previous round is not settled yet")]
    SpinPending,
    #[msg("No pending round")]
    NoPending,
    #[msg("Randomness is already revealed")]
    RandomnessAlreadyRevealed,
    #[msg("Wrong randomness account")]
    WrongRandomness,
    #[msg("Randomness is not resolved yet")]
    RandomnessNotResolved,
    #[msg("Too early: wait for the next slot")]
    SettleTooEarly,
    #[msg("Round has not expired yet")]
    NotExpired,
    #[msg("Round has expired")]
    PendingExpired,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Invalid amount")]
    BadAmount,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Invalid parameters")]
    BadParams,
    #[msg("Mini pool is already seeded")]
    MiniAlreadySeeded,
    #[msg("Session has expired")]
    SessionExpired,
    #[msg("Session limit exceeded")]
    SessionLimit,
    #[msg("Invalid session parameters")]
    BadSession,
    #[msg("Unknown game mode")]
    BadMode,
    #[msg("Withdrawal is locked after a recent deposit")]
    WithdrawLocked,
    #[msg("New bets are paused")]
    Paused,
    #[msg("Round was not sealed in time")]
    NotSealed,
    #[msg("Solvency guard: vault would not cover obligations")]
    SolvencyGuard,
    #[msg("Round secret does not match the committed root")]
    BadProof,
    #[msg("Round secrets are not set or exhausted")]
    HouseExhausted,
    // ---- upgrade No. 2: new errors are added only at the end (the numbers of the existing ones do not shift) ----
    #[msg("Unwrap account check failed")]
    UnwrapFailed,
    #[msg("Only the root authority can do this")]
    NotRootAuthority,
}
