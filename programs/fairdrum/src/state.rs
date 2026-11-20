use anchor_lang::prelude::*;

/// Launch parameters. Set once in initialize and never changed by anyone afterwards.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitParams {
    pub dev_authority: Pubkey,
    pub promo_authority: Pubkey,
    /// Time from which spinning is allowed (registration is open right after launch)
    pub start_ts: i64,
    pub init_act: u64,
    pub init_prize: u64,
    pub init_seed_reserve: u64,
    pub init_jackpot: u64,
    pub init_super: u64,
    pub prize_buffer: u64,
}

#[account]
#[derive(InitSpace, Default)]
pub struct Config {
    pub bump: u8,
    pub vault_bump: u8,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub dev_authority: Pubkey,
    pub promo_authority: Pubkey,
    pub start_ts: i64,
    pub funded: bool,
    /// Last issued registration number
    pub registered: u64,

    // ---- pools (all held in the single vault, this is accounting only) ----
    pub act_pool: u64,
    pub prize_pool: u64,
    pub jackpot_pool: u64,
    pub mini_pool: u64,
    /// Super jackpot (accumulates for a long time, see consts.rs)
    pub super_pool: u64,
    /// No chance of the super jackpot before this time
    /// Left over from the old super model with a time lock; always equals start_ts, not read (AUDIT O14)
    pub super_unlock_ts: i64,
    pub seed_pool: u64,
    pub seed_reserve: u64,
    pub dev_pool: u64,
    pub promo_pool: u64,
    pub prize_buffer: u64,

    // ---- what is expected at funding ----
    pub init_act: u64,
    pub init_prize: u64,
    pub init_seed_reserve: u64,
    pub init_jackpot: u64,
    pub init_super: u64,

    // ---- win plan week ----
    pub den: u64,
    pub epoch_start: i64,
    pub epoch_spins: u64,

    // ---- statistics ----
    pub total_spins: u64,
    pub total_burned: u64,
    pub jackpots_won: u64,
    pub minis_won: u64,
    pub supers_won: u64,

    // ---- pending change of the jackpot starting amounts (takes effect at starts_apply_ts; 0 — no change) ----
    pub next_jackpot: u64,
    pub next_super: u64,
    pub starts_apply_ts: i64,
    // ---- round counters per game: public popularity statistics, does not affect settlements ----
    pub mode_rounds: [u64; 16],
    // ---- public statistics (does not affect settlements): turnover, payouts, administrator's share, recent pot wins ----
    /// sum of paid bets over all time
    pub staked_total: u64,
    /// credited to players over all time: prizes and pots (including jackpot parts not yet paid out)
    pub paid_total: u64,
    /// credited to the administrator over all time
    pub dev_total: u64,
    /// recent pot wins, circular; the next one is written to slot wins_head
    pub wins: [WinRec; 8],
    pub wins_head: u8,
    /// New administrator key proposed by the current one; takes effect when the new key confirms it itself (accept_dev_authority)
    pub pending_dev: Pubkey,
    /// Acceptance of new bets is paused. Settlements, payouts and withdrawals do not depend on this flag.
    pub paused: bool,
    /// "Guardian": a key that can do only ONE thing — pause acceptance of new bets (not lift the pause, and nothing else).
    /// The keeper holds it to stop bets on its own if the vault holds less than the obligations.
    pub guardian: Pubkey,
    /// Obligations to players: the sum of game balances, frozen jackpot parts and bets of unfinished rounds.
    /// Maintained on every change of a player account; the contract uses it to check that the vault covers everything it owes.
    pub owed: u64,
    /// "Fair draw": the root of the tree of house secrets created in advance. Each round's secret is revealed at settlement
    /// and checked against this root, so it cannot be swapped after the bet.
    pub house_root: [u8; 32],
    /// tree depth (it holds 2^depth secrets); 0 — the root is not set yet, bets are not accepted
    pub house_depth: u8,
    /// number of the bet from which the current root took effect
    pub house_start: u64,
    /// running bet counter: each bet gets the next secret number
    pub round_counter: u64,
    /// the eight previous secret roots: none of them can be set again (their secrets have already been revealed)
    pub house_prev: [[u8; 32]; 8],
    /// where in house_prev to write the next previous root
    pub house_prev_head: u8,
    /// how many rounds were closed by a bet refund (the house did not complete the round) — a public reliability indicator
    pub refunds: u64,
}

#[account]
#[derive(InitSpace, Default)]
pub struct Player {
    pub owner: Pubkey,
    pub bump: u8,
    pub reg_no: u64,
    pub registered_at: i64,

    pub balance: u64,
    pub jp_locked: u64,
    pub jp_per_week: u64,
    pub jp_clock: i64,

    pub last_day: i64,
    pub streak: u32,
    pub run_spins: u64,
    pub total_spins: u64,
    pub act_days: u32,
    /// Time of the last deposit (Unix sec.) — for reference. The withdrawal lock is computed from dep_locked / dep_unlock_ts.
    /// The field occupies the place of the former week_idx (weeks of free spins, which no longer exist): the mode is packed into the upper bits of p_n.
    pub last_deposit_ts: i64,
    pub week_used: u16,

    pub pending: bool,
    pub p_n: u16,
    pub p_free: bool,
    pub p_commit_slot: u64,
    /// Root of the house secret tree in effect at the moment of the bet (32 bytes, stored as a key). The round secret
    /// is checked against it: nobody can change the secrets of a bet already placed.
    pub p_root: Pubkey,
    pub p_stake: u64,

    pub won_total: u64,

    /// Number of this round's secret in the house secret tree (the ordinal of the bet since the root was set).
    pub p_leaf: u64,

    /// Starting amounts of the jackpot and the super jackpot in effect at the moment of the bet: this round's chances are computed from them.
    /// The administrator cannot change the chances of a bet already placed (audit of 11.11.2026, K1).
    pub p_start_jackpot: u64,
    pub p_start_super: u64,
    /// cap of a single prize, fixed at the moment of the bet (a quarter of the prize bank at that moment)
    pub p_prize_cap: u64,
    /// The round's "salt": the hash of the block in which the bet is recorded. Nobody knows it at bet time (the block is not yet closed),
    /// so the outcome cannot be computed in advance even knowing the round secret.
    pub p_salt: [u8; 32],
    /// the salt is recorded (by the spin_seal command or by the settlement itself)
    pub p_sealed: bool,
    /// The withdrawal lock applies only to the DEPOSITED amount: this much of the balance cannot be withdrawn before dep_unlock_ts.
    /// Winnings and everything above this amount can be withdrawn immediately.
    pub dep_locked: u64,
    /// When the lock is lifted (Unix sec.). A new deposit shifts the term in proportion to its share of the frozen
    /// amount: someone else's "penny" sent to the deposit address barely changes the term.
    pub dep_unlock_ts: i64,
}

// ---------------- events ----------------
/// Crediting from the personal deposit address: amount — to the game balance, drip — to the player's wallet for network fees
#[event]
pub struct DepositSwept {
    pub owner: Pubkey,
    pub amount: u64,
    pub drip: u64,
    pub payer: Pubkey,
    /// the game account was created by this same crediting (the network's rent for the record was reimbursed to the sender from the deposit)
    pub created: bool,
}
#[event]
pub struct Registered {
    pub owner: Pubkey,
    pub number: u64,
}
#[event]
pub struct Burned {
    pub owner: Pubkey,
    pub streak: u32,
    pub run_spins: u64,
}
#[event]
pub struct SpinCommitted {
    pub owner: Pubkey,
    pub n: u16,
    pub free: bool,
    pub slot: u64,
}
#[event]
pub struct SpinSettled {
    pub owner: Pubkey,
    pub spins: u64,
    pub extras: u64,
    pub gold: u32,
    pub prizes: u64,
    pub best_prize: u64,
    pub mini_hits: u32,
    pub mini_won: u64,
    pub jackpot_hits: u32,
    pub jackpot_total: u64,
    pub super_hits: u32,
    pub super_total: u64,
    pub forfeited: bool,
    /// the house did not complete the round: the bet was returned to the game balance in full
    pub refunded: bool,
}
/// Round reveal: with these three values anyone can verify the round number.
/// number = sha256("fairdrum-round-salt-v1", secret, salt); secret is checked against the root `root` (leaf number `leaf`).
#[event]
pub struct RoundRevealed {
    pub owner: Pubkey,
    pub leaf: u64,
    pub root: [u8; 32],
    pub secret: [u8; 32],
    pub salt: [u8; 32],
    pub commit_slot: u64,
}
/// Inputs of a round's settlement (upgrade No. 2): together with the round number from RoundRevealed this is enough to recompute
/// every outcome and every amount of the round bit for bit (recompute-v2). Written in the same transaction, before the settlement.
/// Amounts are in lamports. `mode` — the game (0…15), `n` — the number of paid bets, `late` — settlement past the deadline (the pots do not pay).
/// `prize_cap`, `start_jackpot`, `start_super` are recorded at bet time; the pools and `init_*` are the config state at the start of the settlement
/// (`init_*` — the starting amounts in effect in this settlement, taking a pending change into account if it is due).
#[event]
pub struct RoundInputs {
    pub owner: Pubkey,
    pub commit_slot: u64,
    pub mode: u8,
    pub n: u16,
    pub free: bool,
    pub late: bool,
    pub prize_cap: u64,
    pub start_jackpot: u64,
    pub start_super: u64,
    pub prize_pool: u64,
    pub jackpot_pool: u64,
    pub mini_pool: u64,
    pub super_pool: u64,
    pub seed_pool: u64,
    pub seed_reserve: u64,
    pub init_jackpot: u64,
    pub init_super: u64,
}
/// The bank's surplus moved to the seed reserve at the end of a settlement (the rule of `consts::bank_release`): `amount`
/// lamports left the prize bank; `prize_pool` and `seed_reserve` are the two pools after the move. Written only when
/// something moved, after SpinSettled.
#[event]
pub struct BankReleased {
    pub amount: u64,
    pub prize_pool: u64,
    pub seed_reserve: u64,
}
#[event]
pub struct EpochRolled {
    pub den: u64,
    pub moved_to_jackpot: u64,
}
#[event]
pub struct Withdrawn {
    pub owner: Pubkey,
    pub amount: u64,
    pub fee: u64,
}
/// Game session (owner's decision of 11.11.2026): a temporary key that the owner has allowed, once,
/// to make spins on their behalf — without a wallet window for every spin.
/// The key lives in the player's browser; the contract limits it by a lifetime and a bet budget.
/// This key cannot withdraw tokens, change the wallet or spend beyond the limit.
#[account]
#[derive(InitSpace, Default)]
pub struct Session {
    pub owner: Pubkey,
    pub key: Pubkey,
    pub expiry_ts: i64,
    /// Remaining bet budget in minimal units; every commit reduces it by the bet amount
    pub limit: u64,
    pub bump: u8,
}

/// Administrator action: kind 1 — collected the share (a = amount), 2 — new starting amounts (a = jackpot, b = super),
/// 3 — move from the reserve to the prize bank (a = amount), 4 — bet pause (a = 1/0), 5 — new key proposed, 6 — key changed,
/// 7 — guardian appointed, 8 — guardian paused bets, 9 — new secret root set (a = depth, b = from which bet),
/// 10 — root authority named by the upgrade authority (key = the root authority), 11 — hand-over of the root authority
/// proposed (key = the proposed key, zero = cancelled), 12 — root authority handed over (key = the new root authority)
#[event]
pub struct AdminAction {
    pub kind: u8,
    pub a: u64,
    pub b: u64,
    pub key: Pubkey,
}

#[event]
pub struct PromoPaid {
    pub to: Pubkey,
    pub amount: u64,
}

/// Record of a pot win: who, how much, when. kind: 1 mini jackpot, 2 jackpot, 3 super jackpot; 0 — empty slot.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default, InitSpace, Debug, PartialEq)]
pub struct WinRec {
    pub player: Pubkey,
    pub amount: u64,
    pub ts: i64,
    pub kind: u8,
}

/// Root authority: the dedicated key that alone sets the root of the house secrets once this account exists (`set_house_root`).
/// It has no other right in the program. A separate small account at the address derived from the word "root-auth": the layout
/// of `Config`, `Player` and `Session` is not touched. Created (and, while an upgrade authority exists, replaced) by
/// `set_root_authority`; handed over by the key itself in two steps. There is no instruction that closes this account.
#[account]
#[derive(InitSpace, Default)]
pub struct RootAuth {
    /// The key that signs `set_house_root`
    pub authority: Pubkey,
    /// Proposed next key (zero = no hand-over in progress); it takes over by signing `accept_root_authority`
    pub pending: Pubkey,
    pub bump: u8,
}
