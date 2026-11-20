//! All game logic as pure state transitions. There are no Solana calls here, so every function can be
//! run with a plain `cargo test`: token conservation, chances, return to players, day expiry, and so on.

use crate::consts::*;
use crate::errors::FdError;
use crate::math::*;
use crate::state::*;
use anchor_lang::prelude::*;

fn mul_bps(v: u64, bps: u64) -> u64 {
    ((v as u128) * (bps as u128) / (BPS as u128)) as u64
}

// ---------------- account: streak, rank, expiry ----------------

/// The rank as others will see it on this day: if a day was missed, the streak has already expired
pub fn effective_run(p: &Player, day: i64) -> u64 {
    if p.last_day != 0 && day - p.last_day > 1 {
        0
    } else {
        p.run_spins
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Touch {
    pub burned: bool,
    pub old_streak: u32,
    pub old_run: u64,
}

/// Any player action. At least one day missed: the streak and the rank are reset to zero.
/// The balance, the registration number and the invitees remain.
pub fn touch(p: &mut Player, now: i64) -> Touch {
    let day = day_of(now);
    let mut t = Touch::default();
    release_jackpot(p, now);
    if p.last_day != 0 && day - p.last_day > 1 {
        t.burned = p.streak > 0 || p.run_spins > 0;
        t.old_streak = p.streak;
        t.old_run = p.run_spins;
        p.streak = 0;
        p.run_spins = 0;
    }
    if p.last_day != day {
        p.streak = if p.last_day != 0 && day - p.last_day == 1 { p.streak + 1 } else { 1 };
        p.last_day = day;
        p.act_days += 1;
    }
    t
}

/// Payout of the frozen part of a jackpot: 8% of the winner's share per week
pub fn release_jackpot(p: &mut Player, now: i64) -> u64 {
    if p.jp_locked == 0 {
        return 0;
    }
    let weeks = ((now - p.jp_clock).max(0) / WEEK_SECS) as u128;
    if weeks == 0 {
        return 0;
    }
    let mut give = (p.jp_per_week as u128 * weeks).min(p.jp_locked as u128) as u64;
    if p.jp_locked - give < 10 {
        give = p.jp_locked; // rounding dust
    }
    p.jp_locked -= give;
    p.balance += give;
    p.jp_clock += (weeks as i64) * WEEK_SECS;
    if p.jp_locked == 0 {
        p.jp_per_week = 0;
    }
    give
}

// ---------------- config: funding, week ----------------

pub fn fund(cfg: &mut Config, vault_amount: u64) -> Result<()> {
    require!(!cfg.funded, FdError::AlreadyFunded);
    let need = (cfg.init_act as u128) + cfg.init_prize as u128 + cfg.init_seed_reserve as u128 + cfg.init_jackpot as u128 + cfg.init_super as u128;
    require!((vault_amount as u128) >= need, FdError::VaultUnderfunded);
    cfg.act_pool = cfg.init_act;
    cfg.prize_pool = cfg.init_prize;
    cfg.seed_reserve = cfg.init_seed_reserve;
    cfg.jackpot_pool = cfg.init_jackpot;
    cfg.super_pool = cfg.init_super;
    // Mini seed: the mini pot never starts from zero (from the seed reserve, O21)
    let m = if cfg.init_jackpot > 0 { mini_start(cfg.init_jackpot).min(cfg.seed_reserve) } else { 0 };
    cfg.mini_pool = m;
    cfg.seed_reserve -= m;
    // starting amounts are never below the minimums: the chance and the top-up after a win are computed from the same number
    cfg.init_jackpot = jackpot_start(cfg.init_jackpot);
    cfg.init_super = super_start(cfg.init_super);
    // The field is left over from the old time-lock model; the lock no longer exists (AUDIT O14), the field is not read.
    cfg.super_unlock_ts = cfg.start_ts;
    // any excess someone sent to the vault goes to the promotion fund
    cfg.promo_pool += (vault_amount as u128 - need) as u64;
    cfg.den = DEN_MIN;
    cfg.epoch_start = cfg.start_ts;
    cfg.funded = true;
    Ok(())
}

/// One-off top-up of the mini seed (AUDIT O21). Needed if fund() ran on a program version
/// without the seed: moves from the seed reserve into the mini pot as much as needed for it to hold MINI_SEED.
/// The vault is not touched — funds move between pools inside the contract, the guard's invariant is preserved.
pub fn mini_topup(cfg: &mut Config) -> Result<u64> {
    require!(cfg.funded, FdError::NotFunded);
    let start = mini_start(cfg.init_jackpot);
    require!(cfg.mini_pool < start, FdError::MiniAlreadySeeded);
    let add = (start - cfg.mini_pool).min(cfg.seed_reserve);
    require!(add > 0, FdError::MiniAlreadySeeded);
    cfg.mini_pool += add;
    cfg.seed_reserve -= add;
    Ok(add)
}

/// The administrator moves funds from the seed reserve into the games' prize bank, only while the bank is below
/// BANK_KEEP and only up to BANK_KEEP (a small bank at launch, or a bank that has been drawn down).
/// No funds are created and none leave the contract.
/// Why the bound (re-verification of 18.11.2026, finding 1): since the bank's surplus feeds the reserve (`bank_release`),
/// a move that left the bank ABOVE BANK_KEEP would be the release run backwards. Repeated after each settlement it would
/// cancel the release for ever: the bank would grow without limit again and no pot would be topped up. With the bound the
/// bank holds at most BANK_KEEP after a move, so the release can never be undone: everything the bank gains above
/// BANK_KEEP afterwards leaves it again at the next settlements. What a move can hold back is the bank's shortfall below
/// BANK_KEEP at that moment, once, and the bank's own gain would have filled that shortfall before any release anyway.
/// Moving into the jackpot or the super jackpot is not allowed (audit No. 3, S1): a pot pays the winner what it holds at the moment
/// of settlement, and the administrator, seeing a jackpot in their own unfinished round, could pour the reserve into it and take it.
/// The pots are replenished from the reserve only by themselves — after a win, up to the starting amount.
pub fn rescale(cfg: &mut Config, to_prize: u64, to_jackpot: u64, to_super: u64) -> Result<()> {
    require!(cfg.funded, FdError::NotFunded);
    require!(to_jackpot == 0 && to_super == 0, FdError::BadAmount);
    require!(to_prize > 0 && to_prize <= cfg.seed_reserve, FdError::BadAmount);
    let bank = cfg.prize_pool.checked_add(to_prize).ok_or(FdError::MathOverflow)?;
    require!(bank <= BANK_KEEP, FdError::BadAmount);
    cfg.seed_reserve -= to_prize;
    cfg.prize_pool = bank;
    Ok(())
}

/// Once a week: `den` = spins over the previous week (statistics). This function moves no funds. The surplus of the prize
/// bank above BANK_KEEP leaves it at settlements, to the seed reserve (`bank_release`). It used to be poured into the jackpot; with a constant jackpot chance that
/// produced a game with positive expectation and made it possible to pump the bank into the pot (audit of 11.11.2026, K2 and S1).
pub fn roll_epoch_if_due(cfg: &mut Config, now: i64) -> Option<EpochRolled> {
    if !cfg.funded || now < cfg.epoch_start + EPOCH_SECS {
        return None;
    }
    cfg.den = cfg.epoch_spins.max(DEN_MIN);
    cfg.epoch_spins = 0;
    let k = (now - cfg.epoch_start) / EPOCH_SECS;
    cfg.epoch_start += k * EPOCH_SECS;
    Some(EpochRolled { den: cfg.den, moved_to_jackpot: 0 })
}

/// Pending starting amounts take effect once their time has come
pub fn apply_starts_if_due(cfg: &mut Config, now: i64) {
    if cfg.starts_apply_ts != 0 && now >= cfg.starts_apply_ts {
        cfg.init_jackpot = cfg.next_jackpot;
        cfg.init_super = cfg.next_super;
        cfg.starts_apply_ts = 0;
    }
}

// ---------------- root of the house secrets ----------------

/// Setting the root of the secret tree (the permission check is in lib.rs). A root that has already been in effect cannot be set
/// a second time: secret numbering would start over, while the secrets of that root were already revealed in past rounds.
/// The same checks and the same writes as in the live program (there they sat directly in the set_house_root command);
/// moved here only so that ordinary tests can check them. There is no delay for a new root (upgrade No. 2, change D).
pub fn set_root(cfg: &mut Config, root: [u8; 32], depth: u8) -> Result<()> {
    require!(depth >= 1 && depth <= HOUSE_MAX_DEPTH && root != [0u8; 32], FdError::BadParams);
    require!(root != cfg.house_root && !cfg.house_prev.contains(&root), FdError::BadParams);
    if cfg.house_depth > 0 {
        let i = (cfg.house_prev_head as usize) % cfg.house_prev.len();
        cfg.house_prev[i] = cfg.house_root;
        cfg.house_prev_head = ((i + 1) % cfg.house_prev.len()) as u8;
    }
    cfg.house_root = root;
    cfg.house_depth = depth;
    cfg.house_start = cfg.round_counter;
    Ok(())
}

// ---------------- registration ----------------

/// Registration is open to everyone right after launch. The number is needed for statistics and for the batch size.
pub fn assign_number(cfg: &mut Config) -> Result<u64> {
    require!(cfg.funded, FdError::NotFunded);
    cfg.registered += 1;
    Ok(cfg.registered)
}

pub fn init_player(p: &mut Player, owner: Pubkey, bump: u8, number: u64, now: i64) {
    *p = Player::default();
    p.owner = owner;
    p.bump = bump;
    p.reg_no = number;
    p.registered_at = now;
    p.last_day = day_of(now);
    p.streak = 1;
    p.act_days = 1;
}

// ---------------- spin: commit ----------------

pub fn commit(cfg: &mut Config, p: &mut Player, now: i64, slot: u64, n: u16, free: bool, randomness: Pubkey) -> Result<Touch> {
    commit_mode(cfg, p, now, slot, n, 0, free, randomness)
}

/// Commit with a game mode: the mode changes only the prize table (the expected payout is equal across all modes),
/// large batches are allowed only for the drum (mode 0).
pub fn commit_mode(cfg: &mut Config, p: &mut Player, now: i64, slot: u64, n: u16, mode: u8, free: bool, randomness: Pubkey) -> Result<Touch> {
    require!(cfg.funded, FdError::NotFunded);
    require!(now >= cfg.start_ts, FdError::NotStarted);
    require!(!p.pending, FdError::SpinPending);
    require!(n >= 1, FdError::BadCount); // the smallest round MIN_BATCH is checked by the spin_commit command itself (lib.rs)
    require!((mode as usize) < MODES, FdError::BadMode);
    require!(n <= batch_limit_mode(mode, cfg.registered), FdError::BatchTooLarge);
    roll_epoch_if_due(cfg, now);
    apply_starts_if_due(cfg, now);
    let t = touch(p, now);
    let stake = (n as u64) * COST;
    if free {
        let allot = FREE_PER_WEEK as u32 + streak_bonus(p.streak) as u32;
        require!(p.week_used as u32 + n as u32 <= allot, FdError::NoFreeSpins);
        require!(cfg.act_pool >= stake, FdError::ActivityFundEmpty);
        cfg.act_pool -= stake;
        p.week_used += n;
    } else {
        require!(p.balance >= stake, FdError::InsufficientBalance);
        p.balance -= stake;
    }
    cfg.mode_rounds[mode as usize] = cfg.mode_rounds[mode as usize].saturating_add(1);
    p.p_prize_cap = cfg.prize_pool / PRIZE_BANK_DIV;
    if !free {
        cfg.staked_total = cfg.staked_total.saturating_add(stake);
    }
    p.pending = true;
    p.p_n = pack_pending(n, mode); // the mode is packed into the upper bits: the account layout did not change
    p.p_free = free;
    p.p_commit_slot = slot;
    p.p_root = randomness;
    p.p_stake = stake;
    // the pot chances of this round are fixed now: nobody can change them later
    p.p_start_jackpot = cfg.init_jackpot;
    p.p_start_super = cfg.init_super;
    Ok(t)
}

// ---------------- spin: settlement ----------------

#[derive(Default, Debug, Clone)]
pub struct SettleOut {
    pub spins: u64,
    pub extras: u64,
    pub gold: u32,
    pub prizes: u64,
    pub best_prize: u64,
    pub mini_hits: u32,
    pub mini_won: u64,
    pub jp_hits: u32,
    pub jp_pool_total: u64,
    pub jp_to_player_now: u64,
    pub super_hits: u32,
    /// how much the player received from the super jackpot (their share is 90%, of which 20% immediately, the rest in parts)
    pub super_total: u64,
    pub burn: u64,
    pub forfeited: bool,
    pub refunded: bool,
    /// moved from the prize bank to the seed reserve at the end of this settlement (`bank_release`)
    pub bank_released: u64,
}

/// The player won `win`: part goes to the balance immediately, the rest is frozen and paid out week by week.
/// Returns the amount paid immediately.
fn grant_vested(p: &mut Player, win: u64, now: i64) -> u64 {
    let now_part = mul_bps(win, JP_NOW_BPS);
    let vest = win - now_part;
    if p.jp_locked == 0 {
        p.jp_clock = now;
    }
    p.jp_locked += vest;
    p.jp_per_week += mul_bps(win, JP_WEEKLY_BPS);
    now_part
}

/// Bet refund: the round has no outcome and never will (the hash of the bet's block was not recorded in time), or the house did not
/// reveal the secret for 24 hours. The player chooses nothing here: they do not know the secret and cannot obstruct settlement.
/// The bet is returned to the game balance in full; nothing from it goes to the pools or to the administrator. Every refund is counted
/// in a public counter: it is a sign, visible to everyone, that the house did not complete a round.
pub fn refund(cfg: &mut Config, p: &mut Player, now: i64) -> Result<SettleOut> {
    require!(p.pending, FdError::NoPending);
    roll_epoch_if_due(cfg, now);
    apply_starts_if_due(cfg, now);
    release_jackpot(p, now);
    let stake = p.p_stake;
    if p.p_free {
        // free spin: the bet came from the activity fund — it goes back there, the weekly counter is rolled back
        cfg.act_pool = cfg.act_pool.checked_add(stake).ok_or(FdError::MathOverflow)?;
        p.week_used = p.week_used.saturating_sub(pending_n(p.p_n));
    } else {
        p.balance = p.balance.checked_add(stake).ok_or(FdError::MathOverflow)?;
        cfg.staked_total = cfg.staked_total.saturating_sub(stake);
    }
    p.pending = false;
    p.p_stake = 0;
    cfg.refunds = cfg.refunds.saturating_add(1);
    Ok(SettleOut { refunded: true, ..Default::default() })
}

/// Inputs of a round's settlement — everything the outcome depends on except the round number (that is published by the RoundRevealed event).
/// Published by the RoundInputs event in the same transaction as the settlement: a third party recomputes the round's prizes
/// bit for bit without trusting the site. The function changes nothing: the values are read exactly as they will be seen by
/// `settle_inner` called right after it with the same `now`.
///  * `init_jackpot`, `init_super` — the starting amounts that will be in effect in the settlement: if a pending change
///    is due, the settlement applies it first (`apply_starts_if_due`), so the new ones are taken here already;
///  * pools — as of the start of the settlement (before this round's shares and payouts);
///  * `prize_cap`, `start_jackpot`, `start_super`, `mode`, `n`, `free` — recorded in the player's account at bet time.
/// `roll_epoch_if_due` and `release_jackpot`, which the settlement calls first, do not touch these fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettleInputs {
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
pub fn round_inputs(cfg: &Config, p: &Player, now: i64, late: bool) -> SettleInputs {
    let due = cfg.starts_apply_ts != 0 && now >= cfg.starts_apply_ts;
    SettleInputs {
        mode: pending_mode(p.p_n),
        n: pending_n(p.p_n),
        free: p.p_free,
        late,
        prize_cap: p.p_prize_cap,
        start_jackpot: p.p_start_jackpot,
        start_super: p.p_start_super,
        prize_pool: cfg.prize_pool,
        jackpot_pool: cfg.jackpot_pool,
        mini_pool: cfg.mini_pool,
        super_pool: cfg.super_pool,
        seed_pool: cfg.seed_pool,
        seed_reserve: cfg.seed_reserve,
        init_jackpot: if due { cfg.next_jackpot } else { cfg.init_jackpot },
        init_super: if due { cfg.next_super } else { cfg.init_super },
    }
}

/// seed = Some(32 bytes of randomness) — a normal settlement; None — the round is closed without a result (not used in this version: an uncompleted round refunds the bet)
pub fn settle(cfg: &mut Config, p: &mut Player, seed: Option<[u8; 32]>, now: i64) -> Result<SettleOut> {
    settle_inner(cfg, p, seed, now, true)
}
/// A settlement more than 20 minutes late. Regular table prizes are paid, the pots are not played: otherwise
/// the owner of the secret could wait for a pot to grow. The flip side: only they can delay a settlement, and that is visible on-chain.
pub fn settle_late(cfg: &mut Config, p: &mut Player, seed: [u8; 32], now: i64) -> Result<SettleOut> {
    settle_inner(cfg, p, Some(seed), now, false)
}
/// Records a pot win in the circular list of recent wins (public statistics).
fn note_win(cfg: &mut Config, player: Pubkey, kind: u8, amount: u64, ts: i64) {
    let i = (cfg.wins_head as usize) % cfg.wins.len();
    cfg.wins[i] = WinRec { player, amount, ts, kind };
    cfg.wins_head = ((i + 1) % cfg.wins.len()) as u8;
}

fn settle_inner(cfg: &mut Config, p: &mut Player, seed: Option<[u8; 32]>, now: i64, pots_open: bool) -> Result<SettleOut> {
    require!(p.pending, FdError::NoPending);
    roll_epoch_if_due(cfg, now);
    apply_starts_if_due(cfg, now);
    release_jackpot(p, now);

    let n = pending_n(p.p_n) as u64;
    let mode = pending_mode(p.p_n);
    let paid = !p.p_free;
    let mut out = SettleOut { forfeited: seed.is_none(), ..Default::default() };
    let mut rng = seed.map(Rng::new);
    let cap = 2 * n + 16; // Consecutive "Again" is capped, the probability of hitting the cap is negligible

    let (mut jack, mut mini, mut prize, mut sd, mut reserve, mut sup) =
        (cfg.jackpot_pool, cfg.mini_pool, cfg.prize_pool, cfg.seed_pool, cfg.seed_reserve, cfg.super_pool);
    let (mut todo, mut exec) = (n, 0u64);
    let mut player_gain = 0u64;
    let mut awarded = 0u64; // everything credited to the player in this round, including the frozen part of a jackpot (for statistics)
    let ob = OddsBase::new_mode(p.p_start_jackpot, p.p_start_super, mode);
    // the largest amounts a pot win is computed from: fixed by the starting amounts recorded at bet time, like the chances
    let cap_jack = pay_cap(jackpot_start(p.p_start_jackpot), JP_PAY_CAP_BPS);
    let cap_mini = pay_cap(mini_start(p.p_start_jackpot), MINI_PAY_CAP_BPS);
    let cap_super = pay_cap(super_start(p.p_start_super), SUPER_PAY_CAP_BPS);

    if rng.is_none() {
        // The spin expired without a result: there are no wins, all shares simply go into the pools. Computed at once for the whole batch,
        // not spin by spin: otherwise an expired spin of a large batch might not fit into the compute limit
        // and would stay unfinished forever.
        // The whole bet of an expired round goes to the players' prize bank: the administrator gets nothing from it (audit No. 3, M2).
        prize += n * COST;
        exec = n;
        todo = 0;
    }

    while todo > 0 {
        todo -= 1;
        exec += 1;
        jack += share(S_JACK);
        mini += share(S_MINI);
        sup += share(S_SUPER);
        sd += share(S_SEED);
        prize += share(S_PRIZE);
        // win plan: the chances depend on the weekly volume and on how much has accumulated in the pots;
        // the super jackpot is open only on paid spins and not before its time (see consts.rs)
        // On a late settlement (pots_open = false) the sector layout is the same as for an on-time settlement, but the pot sectors
        // pay nothing: a late settlement is never more profitable than a timely one, there is no reason to pick the moment (audit No. 3, M1).
        let o = ob.at_super(jack, mini, sup, paid);
        let key = match rng.as_mut() {
            Some(r) => draw_main(r.next_u32(), &o),
            None => Main::Empty,
        };
        match key {
            Main::Empty => {}
            Main::Again => {
                // a free spin is paid for by the prize pool
                if exec < cap && prize >= COST {
                    prize -= COST;
                    todo += 1;
                    out.extras += 1;
                }
            }
            Main::Gold => {
                out.gold += 1;
                let (u1, u2) = rng.as_ref().unwrap().gold(exec as u32);
                match draw_gold_mode(u1, u2, &o, mode) {
                    Gold::Super | Gold::Jackpot | Gold::Mini if !pots_open => {}
                    Gold::Super => {
                        // the win is computed from at most SUPER_PAY_CAP_BPS of the amount recorded at bet time (K2); the rest of the
                        // pot — at least 10% — stays as the beginning of the next super jackpot. No top-up from the seed reserve
                        // (upgrade No. 2): the pot accumulates again from the bets.
                        let win = mul_bps(sup.min(cap_super), SUPER_WINNER_BPS);
                        sup -= win;
                        player_gain += grant_vested(p, win, now);
                        awarded += win;
                        note_win(cfg, p.owner, 3, win, now);
                        out.super_hits += 1;
                        out.super_total += win;
                        cfg.supers_won += 1;
                    }
                    Gold::Jackpot => {
                        // the win is computed from at most JP_PAY_CAP_BPS of the starting amount recorded at bet time (K2);
                        // what the pot holds above that stays in the pot
                        let total = jack.min(cap_jack);
                        // a fifth of that amount stays in the pot as the beginning of the next jackpot (JP_KEEP_BPS); the winner
                        // and the super jackpot share the rest
                        let stay = mul_bps(total, JP_KEEP_BPS);
                        let win = mul_bps(total - stay, JP_WINNER_BPS);
                        let to_super = mul_bps(total - stay, JP_SUPER_BPS);
                        // what leaves the pot; a lamport lost to rounding stays in the pot (nothing goes to the promotion fund)
                        let paid = win + to_super;
                        let keep = jack - paid;
                        sup += to_super;
                        let now_part = grant_vested(p, win, now);
                        player_gain += now_part;
                        out.jp_to_player_now += now_part;
                        awarded += win;
                        note_win(cfg, p.owner, 2, win, now);
                        out.jp_hits += 1;
                        out.jp_pool_total += paid; // what left the pot: the winner's 95% and the super jackpot's 5%
                        // new jackpot: what stayed + the accumulated seed + a top-up from the reserve to the starting amount
                        let base = keep + sd;
                        let top = cfg.init_jackpot.saturating_sub(base).min(reserve);
                        jack = base + top;
                        reserve -= top;
                        sd = 0;
                        cfg.jackpots_won += 1;
                    }
                    Gold::Mini => {
                        // K2: computed from at most MINI_PAY_CAP_BPS of the mini starting amount recorded at bet time; a third of
                        // that amount stays in the pot as the beginning of the next mini (MINI_KEEP_BPS)
                        let total = mini.min(cap_mini);
                        let win = total - mul_bps(total, MINI_KEEP_BPS);
                        let keep = mini - win;
                        player_gain += win;
                        awarded += win;
                        note_win(cfg, p.owner, 1, win, now);
                        out.mini_won += win;
                        out.mini_hits += 1;
                        // the new mini: what stayed + a top-up from the seed reserve to the starting amount (a tenth of the starting jackpot)
                        let top = mini_start(cfg.init_jackpot).saturating_sub(keep).min(reserve);
                        mini = keep + top;
                        reserve -= top;
                        cfg.minis_won += 1;
                    }
                    Gold::Prize(tokens) => {
                        // a prize is capped by a share of the prize bank: one win does not leave the bank empty for the others
                        // the cap is a quarter of the bank, and no more than it was at the moment of the bet: the bank cannot be topped up "for an already
                        // known prize" (audit No. 3, S1)
                        let amt = (tokens * ONE).min(prize / PRIZE_BANK_DIV).min(p.p_prize_cap);
                        prize -= amt;
                        player_gain += amt;
                        out.prizes += amt;
                        awarded += amt;
                        out.best_prize = out.best_prize.max(amt);
                    }
                }
            }
        }
    }

    // The bank's surplus goes to the seed reserve: what the bank holds above BANK_KEEP after this settlement, at most
    // BANK_RELEASE_BPS of a bet per executed spin. After the loop, so the round's own prizes and restarts do not depend on it.
    out.bank_released = bank_release(prize, exec);
    prize -= out.bank_released;
    reserve += out.bank_released;

    cfg.jackpot_pool = jack;
    cfg.mini_pool = mini;
    cfg.prize_pool = prize;
    cfg.seed_pool = sd;
    cfg.seed_reserve = reserve;
    cfg.super_pool = sup;

    out.spins = exec;
    let shared = if out.forfeited { 0 } else { exec }; // an expired round has no shares: it is entirely in the prize bank
    out.burn = shared * share(S_BURN);
    cfg.total_burned += out.burn;
    cfg.act_pool += shared * share(S_ACT);
    cfg.dev_pool += shared * share(S_DEV);
    cfg.dev_total = cfg.dev_total.saturating_add(shared * share(S_DEV));
    cfg.paid_total = cfg.paid_total.saturating_add(awarded);

    p.balance += player_gain;
    p.won_total += player_gain;
    p.run_spins += exec;
    p.total_spins += exec;
    p.pending = false;
    p.p_stake = 0;
    cfg.total_spins += exec;
    cfg.epoch_spins += exec;
    Ok(out)
}

// ---------------- deposit and withdrawal ----------------

/// Deposit to the game balance. Remembers the time: withdrawal opens DEPOSIT_LOCK_SECS (zero since upgrade No. 2) after the last deposit.
pub fn deposit(p: &mut Player, amount: u64, now: i64) -> Result<()> {
    require!(amount > 0, FdError::BadAmount);
    p.balance = p.balance.checked_add(amount).ok_or(FdError::MathOverflow)?;
    p.last_deposit_ts = now;
    // The withdrawal lock applies only to what was deposited. The term of the new frozen amount is the amount-weighted average: the remaining term
    // for the previous amount and the full term for the new one. So a deposit of 1% of the frozen amount shifts the term by 1%, not by 12 hours.
    let old = dep_locked_now(p, now);
    let rem_old = if old > 0 { (p.dep_unlock_ts - now).max(0) as u128 } else { 0 };
    let total = old.checked_add(amount).ok_or(FdError::MathOverflow)?;
    let weighted = (old as u128) * rem_old + (amount as u128) * (DEPOSIT_LOCK_SECS as u128);
    let rem = weighted.div_ceil(total as u128).min(DEPOSIT_LOCK_SECS as u128) as i64;
    p.dep_locked = total;
    p.dep_unlock_ts = now + rem;
    Ok(())
}
/// How much of the balance is currently frozen by the withdrawal lock
pub fn dep_locked_now(p: &Player, now: i64) -> u64 {
    // With a zero lock period nothing is ever frozen, including amounts that an earlier program version froze for 12 hours
    // (their `dep_unlock_ts` may still lie in the future right after the upgrade).
    if DEPOSIT_LOCK_SECS == 0 || now >= p.dep_unlock_ts {
        0
    } else {
        p.dep_locked
    }
}
/// How much the player can withdraw right now (not counting the frozen jackpot payout, which is separate)
pub fn withdrawable(p: &Player, now: i64) -> u64 {
    p.balance.saturating_sub(dep_locked_now(p, now))
}
/// Time when the withdrawal lock is lifted (0 — nothing is frozen)
pub fn withdraw_open_ts(p: &Player) -> i64 {
    if p.dep_locked == 0 {
        0
    } else {
        p.dep_unlock_ts
    }
}
/// Donation to the seed reserve: this is how anyone contributes their own funds to the jackpots. It touches neither the
/// prize bank nor BANK_KEEP, so it cannot trigger or enlarge a release of the bank's surplus.
/// Funds cannot be withdrawn back from the reserve — they go only into the pots, which go to players.
pub fn donate(cfg: &mut Config, amount: u64) -> Result<()> {
    require!(cfg.funded, FdError::NotFunded);
    require!(amount > 0, FdError::BadAmount);
    cfg.seed_reserve = cfg.seed_reserve.checked_add(amount).ok_or(FdError::MathOverflow)?;
    Ok(())
}
/// The administrator sets the starting amounts of the jackpot and the super jackpot (a pot is topped up to them from the reserve after a win,
/// and the chance is computed from them). Rules that prevent manipulating this (audit of 11.11.2026, K1 and K2):
/// new amounts take effect only after 24 hours; bets already placed are settled at the previous ones; an amount cannot be
/// below the minimum, above START_MAX, or below half of what the pot already holds (otherwise a pot filled
/// by players could be made easy prey).
/// Upgrade No. 2 (audit No. 9) adds two bounds:
///  * G1 — a DECREASE of the jackpot starting amount cannot drop the mini starting amount (a tenth of it) below half
///    of the accumulated mini pot: otherwise the mini chance grows with the same pot and the round becomes positive-expectation;
///  * G2 — a RAISE in one go is at most fourfold relative to the starting amount in effect or to the pot (whichever is larger): a pot
///    can be won from a quarter of the starting amount, so an open pot stays open after the raise.
/// Both bounds compare against the amounts in effect now (a pending change that has not yet taken effect does not count).
///
/// Every check is PER POT (verification of 18.11.2026, finding 1). An amount equal to the one in effect is "not being
/// changed" and is not validated against its pot at all; the checks of one pot never read the other pot. Before this, a
/// super pot above 2 × START_MAX (or simply above twice its amount) made the command fail whatever was asked, so the
/// jackpot amount could not be changed either until the super jackpot was won. Leaving an amount as it is gives nobody
/// anything: the chance of that pot stays what it already is, and a win is computed from at most the pay cap of the amount
/// recorded at BET time (K2), so no value of either amount can raise the expectation of a spin above the bound of consts.rs.
/// A command that changes neither amount is accepted and only cancels a pending change (it restarts the 24 hours).
pub fn set_starts(cfg: &mut Config, jackpot_start: u64, super_start: u64, now: i64) -> Result<()> {
    require!(cfg.funded, FdError::NotFunded);
    apply_starts_if_due(cfg, now);
    let (cur_jackpot, cur_super) = (jackpot_start_of(cfg), super_start_of(cfg));
    if jackpot_start != cur_jackpot {
        require!(jackpot_start >= JP_MIN_POT && jackpot_start <= START_MAX, FdError::BadParams);
        // K2: not below half of what the pot already holds
        require!(jackpot_start >= cfg.jackpot_pool / 2, FdError::BadParams);
        // G1. Needed only on a decrease: with a larger amount the mini chance does not grow.
        require!(jackpot_start >= cur_jackpot || mini_start(jackpot_start) >= cfg.mini_pool / 2, FdError::BadParams);
        // G2
        require!(jackpot_start <= starts_raise_cap(cfg.jackpot_pool, cur_jackpot), FdError::BadParams);
    }
    if super_start != cur_super {
        require!(super_start >= SUPER_MIN_POT && super_start <= START_MAX, FdError::BadParams);
        require!(super_start >= cfg.super_pool / 2, FdError::BadParams); // K2
        require!(super_start <= starts_raise_cap(cfg.super_pool, cur_super), FdError::BadParams); // G2
    }
    cfg.next_jackpot = jackpot_start;
    cfg.next_super = super_start;
    cfg.starts_apply_ts = now + STARTS_DELAY_SECS;
    Ok(())
}

/// Jackpot / super jackpot starting amount in effect (not below the minimum — the chance computation reads it the same way)
pub fn jackpot_start_of(cfg: &Config) -> u64 {
    jackpot_start(cfg.init_jackpot)
}
pub fn super_start_of(cfg: &Config) -> u64 {
    super_start(cfg.init_super)
}
/// The largest starting amount that can be set in one go (G2): fourfold the larger of "the pot now" and
/// "the starting amount in effect", but not above START_MAX.
pub fn starts_raise_cap(pool: u64, current_start: u64) -> u64 {
    pool.max(current_start).saturating_mul(STARTS_RAISE_MUL).min(START_MAX)
}

/// Withdrawal. Returns (to the player, fee, burned); the SOL version has no project fee and no burning.
/// Withdrawal opens DEPOSIT_LOCK_SECS after the last deposit (zero since upgrade No. 2: at once).
pub fn withdraw(cfg: &mut Config, p: &mut Player, amount: u64, now: i64) -> Result<(u64, u64, u64)> {
    require!(amount > 0, FdError::BadAmount);
    release_jackpot(p, now);
    require!(p.balance >= amount, FdError::InsufficientBalance);
    // the lock applies only to the deposited amount: winnings and everything above it can be withdrawn immediately
    require!(amount <= withdrawable(p, now), FdError::WithdrawLocked);
    if dep_locked_now(p, now) == 0 {
        p.dep_locked = 0;
    }
    let bps = RANKS[rank_idx(effective_run(p, day_of(now)))].1;
    let fee = mul_bps(amount, bps);
    let burn = if BURN_ENABLED { fee / 2 } else { 0 };
    p.balance -= amount;
    cfg.total_burned += burn;
    cfg.jackpot_pool += fee - burn;
    Ok((amount - fee, fee, burn))
}

