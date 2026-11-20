//! The contract sustains itself (owner's decision of 18.11.2026, consts.rs "The contract sustains itself"):
//!  1. a jackpot or mini win leaves a share of the amount it is computed from in the pot (JP_KEEP_BPS, MINI_KEEP_BPS);
//!  2. what the prize bank holds above BANK_KEEP moves to the seed reserve at the end of a settlement, at most
//!     BANK_RELEASE_BPS of a bet per executed spin (`consts::bank_release`).
//! Everything is computed from the program's own constants and functions; amounts are exact integers (lamports).
//!
//!   cargo test --release -p fairdrum --test self_tests -- --nocapture
//!
//! FD_SELF_STAKES=<n> changes the length of the long simulation (default 20 million paid bets).

use anchor_lang::prelude::Pubkey;
use fairdrum::consts::*;
use fairdrum::engine;
use fairdrum::math::*;
use fairdrum::state::*;

const T0: i64 = 1_800_000_000;
const SOL: u64 = 1_000_000 * ONE;
const Q32: u128 = 1 << 32;

struct Sm(u64);
impl Sm {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn seed(&mut self) -> [u8; 32] {
        let mut s = [0u8; 32];
        for i in 0..4 {
            s[i * 8..i * 8 + 8].copy_from_slice(&self.next().to_le_bytes());
        }
        s
    }
}
fn key(i: u64) -> Pubkey {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&i.to_le_bytes());
    b[31] = 5;
    Pubkey::new_from_array(b)
}
fn funded(prize: u64, jackpot: u64, superj: u64, reserve: u64) -> (Config, u64) {
    let mut c = Config::default();
    c.start_ts = T0;
    c.init_prize = prize;
    c.init_seed_reserve = reserve;
    c.init_jackpot = jackpot;
    c.init_super = superj;
    let vault = prize + jackpot + superj + reserve;
    engine::fund(&mut c, vault).unwrap();
    (c, vault)
}
fn player(c: &mut Config, i: u64) -> Player {
    let mut p = Player::default();
    let n = engine::assign_number(c).unwrap();
    engine::init_player(&mut p, key(i), 255, n, T0);
    p
}
/// The sum `solvent()` in lib.rs compares the vault with
fn pools(c: &Config) -> u128 {
    [c.act_pool, c.prize_pool, c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_pool, c.seed_reserve, c.dev_pool, c.promo_pool].iter().map(|x| *x as u128).sum()
}
/// `owed_of` in lib.rs
fn owed(p: &Player) -> u128 {
    p.balance as u128 + p.jp_locked as u128 + if p.pending { p.p_stake as u128 } else { 0 }
}
fn bps(v: u64, b: u64) -> u64 {
    (v as u128 * b as u128 / BPS as u128) as u64
}

// ---------------------------------------------------------------------------------------------------------------------------
// 1. The constants and what follows from them
// ---------------------------------------------------------------------------------------------------------------------------

#[test]
fn constants_and_the_numbers_the_documents_quote() {
    assert_eq!((JP_KEEP_BPS, MINI_KEEP_BPS, BANK_KEEP, BANK_RELEASE_BPS), (2_000, 3_333, 1_600 * SOL, 50));
    assert_eq!(share(BANK_RELEASE_BPS), 50_000, "0.5% of a bet per executed spin");
    assert_eq!(max_table_prize() * ONE, 10 * SOL);
    // a jackpot computed from T: 20% stays, the winner 76%, the super jackpot 4%
    let t = 7 * SOL;
    let stay = bps(t, JP_KEEP_BPS);
    assert_eq!((stay, bps(t - stay, JP_WINNER_BPS), bps(t - stay, JP_SUPER_BPS)), (t / 100 * 20, t / 100 * 76, t / 100 * 4));
    // the bank: buffer in mean depths of every game, and the release against the bank's gain
    let buffer = (BANK_KEEP / ONE - PRIZE_BANK_DIV * max_table_prize()) as f64;
    let (mut worst, mut gain_max, mut gain_min) = (f64::MAX, 0f64, f64::MAX);
    for m in 0..MODES {
        let gain = bank_surplus_ppm(m) as f64 / 1e6; // points per executed spin (a bet = 10,000 points)
        let depth = bank_out_sq_ppm(m) as f64 / 1e6 / (2.0 * gain); // points
        worst = worst.min(buffer / depth);
        gain_max = gain_max.max(gain);
        gain_min = gain_min.min(gain);
        println!("game {m:2}: the bank gains {gain:.3} points per spin ({:.4}% of paid bets), mean depth below the threshold {:.1} SOL, buffer = {:.1} depths", gain / 88.40, depth / 1e6, buffer / depth);
    }
    assert!(worst >= BANK_BUFFER_DEPTHS as f64 && worst < 8.0, "{worst}");
    assert!((gain_min - 28.938).abs() < 0.001 && gain_max < 29.07, "{gain_min} {gain_max}");
    let limit = share(BANK_RELEASE_BPS) as f64 / ONE as f64;
    assert!(limit / gain_max > 1.7 && limit / gain_max < 1.8);
    // long-run share of time below the threshold = (limit − gain) ÷ limit; share of time with the bank below "any prize in full"
    let below = (limit - gain_min) / limit;
    let p = below * (-worst).exp();
    println!("drum only (the worst game): the bank is below the threshold {:.1}% of the time and below 40 SOL about {:.4}% of the time (1 stake in {:.0})", below * 100.0, p * 100.0, 1.0 / p);
    assert!(p < 0.0003);
    // reserve: demand of full restarts against what the bank can release
    let demand = reserve_demand_ppm() as f64 / 1e6;
    println!("top-ups of full restarts ask {demand:.2} points per spin; the bank releases at most {gain_max:.2} on average: ratio {:.3}", demand / gain_max);
    assert!(demand / gain_max > 1.2 && demand / gain_max < 1.3);
    // the per-bet bound with the kept shares, and the old one, which still holds
    let head = (BPS - W_AGAIN) as f64;
    let table = mode_ev_ppm_tokens(0) as f64 / 1e6;
    let old = pots_ev_bound_scaled() as f64 / 1e6 / 1e8;
    let new = pots_ev_kept_bound_scaled() as f64 / 1e6 / 1e12;
    println!("a bet returns at most {:.4}% in any state (pots {new:.3} of 10,000); without the kept shares the bound was {:.4}% (pots {old:.3})", (table + new) / head * 100.0, (table + old) / head * 100.0);
    assert!((new - 947.894).abs() < 0.001 && (old - 1_042.05).abs() < 0.001);
    assert!(((table + new) / head - 0.983932).abs() < 0.000001 && ((table + old) / head - 0.994583).abs() < 0.000001);
    // long-run return to players once the bank is at its threshold: everything except the administrator's share
    let rtp = (BPS - W_AGAIN - S_DEV) as f64 / head;
    assert_eq!((BPS - W_AGAIN - S_DEV, BPS - W_AGAIN), (8_222, 8_840));
    println!("long-run return to players: {:.4}% of paid bets (8,222 ÷ 8,840); while the bank is below the threshold {:.4}%", rtp * 100.0, (table + (S_JACK + S_MINI + S_SEED + S_SUPER) as f64) / head * 100.0);
    assert!((rtp - 0.930090).abs() < 0.000001);
    // with the invite reward (50% of the administrator's share) and the administrator's own play, every pot at its cap
    let worst_with = |extra: f64| (table + new + extra) / head * 100.0;
    println!("worst state: with the invite reward on every round {:.2}%, the administrator's own play {:.2}%; a round the report's rule rewards {:.3}%", worst_with(309.0), worst_with(618.0), (table + 750.0 - (836.0 - 668.8) / 10.0 * (750.0 - 659.25) / 83.6 + 309.0) / head * 100.0);
    assert!((worst_with(309.0) - 101.889).abs() < 0.001 && (worst_with(618.0) - 105.384).abs() < 0.001);
    // the largest invite share for which the worst state stays below the stake
    let max_bps = ((head - table - new) / S_DEV as f64 * 1e4).floor();
    assert_eq!(max_bps as u64, 2_298);
}

#[test]
fn bank_release_is_a_function_of_two_numbers() {
    let lim = share(BANK_RELEASE_BPS);
    assert_eq!(bank_release(0, 1_000), 0);
    assert_eq!(bank_release(BANK_KEEP, 1_000), 0);
    assert_eq!(bank_release(BANK_KEEP - 1, u64::MAX), 0);
    assert_eq!(bank_release(BANK_KEEP + 1, 1), 1);
    assert_eq!(bank_release(BANK_KEEP + lim, 1), lim);
    assert_eq!(bank_release(BANK_KEEP + lim + 1, 1), lim);
    assert_eq!(bank_release(BANK_KEEP + 7 * SOL, 2_016), 2_016 * lim, "the largest round (2 × 1000 + 16 spins) releases 0.1008 SOL at most");
    assert_eq!(bank_release(u64::MAX, 0), 0);
    assert_eq!(bank_release(u64::MAX, u64::MAX), u64::MAX - BANK_KEEP);
    let mut sm = Sm(11);
    for _ in 0..200_000 {
        let (pool, exec) = (sm.below(4_000 * SOL), sm.below(3_000));
        let r = bank_release(pool, exec);
        assert!(r <= exec * lim && r <= pool.saturating_sub(BANK_KEEP) && (r == 0 || pool - r >= BANK_KEEP));
        assert!(r == exec * lim || pool - r <= BANK_KEEP, "either the limit binds or the bank ends at the threshold");
    }
}

// ---------------------------------------------------------------------------------------------------------------------------
// 2. A reference settlement built from the public primitives, with the two rules switchable
// ---------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
struct Rules {
    jp_keep: u64,
    mini_keep: u64,
    release: bool,
}
const NOW: Rules = Rules { jp_keep: JP_KEEP_BPS, mini_keep: MINI_KEEP_BPS, release: true };
/// the prepared upgrade before this change
const BEFORE: Rules = Rules { jp_keep: 0, mini_keep: 0, release: false };

#[derive(Clone, PartialEq, Debug, Default)]
struct Ref {
    /// per executed spin: 0 nothing, 1 repeat taken, 2 repeat not taken, 3 super, 4 jackpot, 5 mini, 6 pot sector of a late round, 10 + amount: table prize
    codes: Vec<u64>,
    pools: [u64; 6], // prize, jackpot, mini, super, seed pool, reserve
    credited: u64,
    locked: u64,
    released: u64,
    pot_wins: u32,
    first_pot_win: usize,
}
fn reference(c: &Config, p: &Player, seed: [u8; 32], late: bool, r: Rules) -> Ref {
    let (n, mode) = (pending_n(p.p_n) as u64, pending_mode(p.p_n));
    let (mut prize, mut jack, mut mini, mut sup, mut sd, mut reserve) = (c.prize_pool, c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_pool, c.seed_reserve);
    let ob = OddsBase::new_mode(p.p_start_jackpot, p.p_start_super, mode);
    let (cj, cm, cs) = (pay_cap(jackpot_start(p.p_start_jackpot), JP_PAY_CAP_BPS), pay_cap(mini_start(p.p_start_jackpot), MINI_PAY_CAP_BPS), pay_cap(super_start(p.p_start_super), SUPER_PAY_CAP_BPS));
    let mut rng = Rng::new(seed);
    let mut out = Ref { first_pot_win: usize::MAX, ..Default::default() };
    let (mut todo, mut exec, cap) = (n, 0u64, 2 * n + 16);
    while todo > 0 {
        todo -= 1;
        exec += 1;
        jack += share(S_JACK);
        mini += share(S_MINI);
        sup += share(S_SUPER);
        sd += share(S_SEED);
        prize += share(S_PRIZE);
        let o = ob.at_super(jack, mini, sup, !p.p_free);
        let code = match draw_main(rng.next_u32(), &o) {
            Main::Empty => 0,
            Main::Again => {
                if exec < cap && prize >= COST {
                    prize -= COST;
                    todo += 1;
                    1
                } else {
                    2
                }
            }
            Main::Gold => {
                let (u1, u2) = rng.gold(exec as u32);
                match draw_gold_mode(u1, u2, &o, mode) {
                    Gold::Super | Gold::Jackpot | Gold::Mini if late => 6,
                    Gold::Super => {
                        let win = bps(sup.min(cs), SUPER_WINNER_BPS);
                        sup -= win;
                        out.credited += bps(win, JP_NOW_BPS);
                        out.locked += win - bps(win, JP_NOW_BPS);
                        3
                    }
                    Gold::Jackpot => {
                        let total = jack.min(cj);
                        let stay = bps(total, r.jp_keep);
                        let (win, to_super) = (bps(total - stay, JP_WINNER_BPS), bps(total - stay, JP_SUPER_BPS));
                        // (before this change the rounding lamport went to the promotion fund; it is not tracked here)
                        let base = jack - if r.jp_keep > 0 { win + to_super } else { total } + sd;
                        sup += to_super;
                        out.credited += bps(win, JP_NOW_BPS);
                        out.locked += win - bps(win, JP_NOW_BPS);
                        let top = c.init_jackpot.saturating_sub(base).min(reserve);
                        jack = base + top;
                        reserve -= top;
                        sd = 0;
                        out.pot_wins += 1;
                        out.first_pot_win = out.first_pot_win.min(out.codes.len());
                        4
                    }
                    Gold::Mini => {
                        let total = mini.min(cm);
                        let win = total - bps(total, r.mini_keep);
                        let keep = mini - win;
                        out.credited += win;
                        let top = mini_start(c.init_jackpot).saturating_sub(keep).min(reserve);
                        mini = keep + top;
                        reserve -= top;
                        out.pot_wins += 1;
                        out.first_pot_win = out.first_pot_win.min(out.codes.len());
                        5
                    }
                    Gold::Prize(tokens) => {
                        let amt = (tokens * ONE).min(prize / PRIZE_BANK_DIV).min(p.p_prize_cap);
                        prize -= amt;
                        out.credited += amt;
                        10 + amt
                    }
                }
            }
        };
        out.codes.push(code);
    }
    if r.release {
        out.released = bank_release(prize, exec);
        prize -= out.released;
        reserve += out.released;
    }
    out.pools = [prize, jack, mini, sup, sd, reserve];
    out
}

fn random_state(sm: &mut Sm, k: u64) -> (Config, Player, bool) {
    let pick = |sm: &mut Sm, v: &[u64]| v[sm.below(v.len() as u64) as usize];
    let j = pick(sm, &[0, JP_MIN_POT, SOL, 5 * SOL, 50 * SOL, START_MAX]);
    let s = pick(sm, &[0, SUPER_MIN_POT, 20 * SOL, 320 * SOL, START_MAX]);
    let (mut c, _) = funded(0, j, s, 0);
    let (js, ms, ss) = (jackpot_start(j), mini_start(j), super_start(s));
    let lvl = |sm: &mut Sm, start: u64| match sm.below(8) {
        0 => 0,
        1 => start / 4 - sm.below(100_000).min(start / 4),
        2 => start / 4 + sm.below(100_000),
        3 => start,
        4 => 3 * start - sm.below(200_000).min(start),
        5 => 3 * start + sm.below(SOL),
        6 => sm.below(4 * start + 1),
        _ => 1_000 * start.min(1_000 * SOL) + sm.below(1_000),
    };
    c.jackpot_pool = lvl(sm, js);
    c.mini_pool = lvl(sm, ms);
    c.super_pool = lvl(sm, ss);
    c.seed_pool = pick(sm, &[0, 12_345, js / 5, js]);
    c.seed_reserve = pick(sm, &[0, 1, ms / 3, js / 2, 30 * SOL, 5_000 * SOL]);
    c.prize_pool = match sm.below(8) {
        0 => sm.below(SOL),
        1 => 30 * SOL + sm.below(20 * SOL),
        2 => 200 * SOL,
        3 => BANK_KEEP - sm.below(3 * SOL),
        4 => BANK_KEEP + sm.below(3 * SOL),
        5 => BANK_KEEP + sm.below(10_000_000),
        6 => 63_000 * SOL,
        _ => sm.below(3_000 * SOL),
    };
    let mut p = player(&mut c, k);
    let mode = sm.below(MODES as u64) as u8;
    let n = if mode == 0 { [10u16, 37, 100, 1000][sm.below(4) as usize] } else { [10u16, 25, 100][sm.below(3) as usize] };
    p.balance = n as u64 * COST;
    engine::commit_mode(&mut c, &mut p, T0 + 1, k + 1, n, mode, false, key(9)).unwrap();
    // between the bet and the settlement the administrator's amounts and the bank may have changed
    if sm.below(4) == 0 {
        c.init_jackpot = pick(sm, &[JP_MIN_POT, 2 * SOL, 40 * SOL]);
        c.prize_pool = c.prize_pool / 2 + sm.below(SOL);
    }
    (c, p, sm.below(9) == 0)
}

/// Random states (empty, closed, at the caps and a thousand starting amounts above them; banks below, at and above
/// BANK_KEEP; reserves from zero to thousands of SOL), every game, late rounds:
///  * the engine equals the reference with both rules on, field for field;
///  * the ledger is exact: pools + what the player is owed change by nothing; the promotion fund gets nothing; the
///    administrator gets exactly S_DEV per executed spin;
///  * compared with the rules BEFORE this change, for the same round number: every outcome and every table prize is the same up
///    to and including the round's first jackpot or mini win (goal E); a round without such a win is identical in every pool
///    except the release, which is exactly `bank_release`.
#[test]
fn random_states_ledger_exact_and_table_prizes_unchanged() {
    let mut sm = Sm(0x5E1F_0001);
    let (mut rounds, mut with_win, mut identical_after_win, mut released_rounds, mut prizes, mut late_n) = (0u32, 0u32, 0u32, 0u32, 0u64, 0u32);
    for k in 0..150_000u64 {
        let (c0, p0, late) = random_state(&mut sm, k);
        let seed = sm.seed();
        let (mut c, mut p) = (c0.clone(), p0.clone());
        let before = pools(&c) + owed(&p);
        let out = if late { engine::settle_late(&mut c, &mut p, seed, T0 + 2).unwrap() } else { engine::settle(&mut c, &mut p, Some(seed), T0 + 2).unwrap() };
        assert_eq!(pools(&c) + owed(&p), before, "round {k}: the ledger");
        assert_eq!(c.promo_pool, c0.promo_pool);
        assert_eq!(c.dev_pool - c0.dev_pool, out.spins * share(S_DEV));
        let now = reference(&c0, &p0, seed, late, NOW);
        assert_eq!([c.prize_pool, c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_pool, c.seed_reserve], now.pools, "round {k}");
        assert_eq!((p.balance, p.jp_locked, out.bank_released, out.spins), (now.credited, now.locked, now.released, now.codes.len() as u64), "round {k}");
        assert_eq!(out.bank_released, bank_release(c.prize_pool + out.bank_released, out.spins));
        assert!(out.bank_released == 0 || c.prize_pool >= BANK_KEEP, "a release never takes the bank below BANK_KEEP");
        // pot wins are within the caps recorded at the bet and the kept shares
        let (cj, cm) = (pay_cap(jackpot_start(p0.p_start_jackpot), JP_PAY_CAP_BPS), pay_cap(mini_start(p0.p_start_jackpot), MINI_PAY_CAP_BPS));
        assert!(out.jp_pool_total <= out.jp_hits as u64 * (cj - bps(cj, JP_KEEP_BPS)));
        assert!(out.mini_won <= out.mini_hits as u64 * (cm - bps(cm, MINI_KEEP_BPS)));
        // against the rules before this change
        let old = reference(&c0, &p0, seed, late, BEFORE);
        let upto = now.first_pot_win.min(now.codes.len().min(old.codes.len()));
        assert_eq!(now.first_pot_win, old.first_pot_win, "round {k}: the same spin is the first pot win");
        assert_eq!(now.codes[..upto.min(now.codes.len())], old.codes[..upto.min(old.codes.len())], "round {k}: outcomes and table prizes before the first pot win");
        if now.pot_wins == 0 {
            assert_eq!(now.codes, old.codes, "round {k}");
            assert_eq!((now.credited, now.locked), (old.credited, old.locked));
            let mut o = old.pools;
            o[0] -= now.released;
            o[5] += now.released;
            assert_eq!(now.pools, o, "round {k}: the only difference is the release");
        } else {
            with_win += 1;
            identical_after_win += (now.codes == old.codes) as u32;
        }
        rounds += 1;
        released_rounds += (out.bank_released > 0) as u32;
        late_n += late as u32;
        prizes += now.codes.iter().filter(|x| **x >= 10).count() as u64;
    }
    println!("{rounds} random rounds ({late_n} late, {prizes} table prizes): ledger exact, engine = reference; {released_rounds} rounds released bank surplus; {with_win} rounds had a jackpot or mini win, {identical_after_win} of them kept every later outcome of the round as before");
    assert!(with_win > 3_000 && released_rounds > 20_000 && late_n > 10_000 && prizes > 1_000_000);
}

// ---------------------------------------------------------------------------------------------------------------------------
// 3. The per-bet bound in any state, exact
// ---------------------------------------------------------------------------------------------------------------------------

/// Expected value of one executed spin, × 2^32, with the program's own chances: table + the three pot wins given.
fn spin_ev_q32(o: &Odds, mode: u8, jw: u64, mw: u64, sw: u64) -> u128 {
    let ladder: u128 = MODE_LADDER[mode as usize].iter().map(|(t, w)| (*t * ONE) as u128 * *w as u128).sum();
    o.prize as u128 * ladder / LADDER_SUM as u128 + o.jackpot as u128 * jw as u128 + o.mini as u128 * mw as u128 + o.superj as u128 * sw as u128
}
#[test]
fn no_state_makes_a_bet_worth_its_stake_with_the_kept_shares() {
    let mut worst = 0f64;
    let mut sm = Sm(77);
    let mut states = 0u32;
    for mode in 0..MODES as u8 {
        for k in 0..25_000u32 {
            let j = [JP_MIN_POT, SOL, 5 * SOL, START_MAX, JP_MIN_POT + sm.below(START_MAX)][k as usize % 5];
            let s = [SUPER_MIN_POT, 20 * SOL, 320 * SOL, START_MAX, SUPER_MIN_POT + sm.below(START_MAX)][(k as usize / 5) % 5];
            let o = OddsBase::new_mode(j, s, mode).at_super(u64::MAX / 4, u64::MAX / 4, u64::MAX / 4, true);
            // the largest wins the settlement can pay: pots far above the caps
            let (cj, cm, cs) = (pay_cap(jackpot_start(j), JP_PAY_CAP_BPS), pay_cap(mini_start(j), MINI_PAY_CAP_BPS), pay_cap(super_start(s), SUPER_PAY_CAP_BPS));
            let (jw, mw, sw) = (bps(cj - bps(cj, JP_KEEP_BPS), JP_WINNER_BPS), cm - bps(cm, MINI_KEEP_BPS), bps(cs, SUPER_WINNER_BPS));
            let ev = spin_ev_q32(&o, mode, jw, mw, sw);
            let den = Q32 - o.again as u128;
            assert!(ev < COST as u128 * den);
            worst = worst.max(ev as f64 / den as f64 / COST as f64);
            states += 1;
        }
    }
    println!("{states} states with every pot above its cap, all games, amounts from the minimum to START_MAX: a bet returns at most {:.5}%", worst * 100.0);
    assert!(worst < 0.98394 && worst > 0.9838);
}

/// The same measured with the engine in the richest state (every pot a thousand starting amounts, bank far above BANK_KEEP so
/// that the release runs in every round): what the player gets, and that the release does not reach him.
#[test]
fn engine_in_the_richest_state_with_the_release_running() {
    let bets: u64 = std::env::var("FD_SELF_RICH_BETS").ok().and_then(|s| s.parse().ok()).unwrap_or(30_000_000);
    let (mut c, _) = funded(1_000_000 * SOL, JP_MIN_POT, SUPER_MIN_POT, 0);
    let mut p = player(&mut c, 1);
    let mut sm = Sm(std::env::var("FD_SELF_RICH_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1));
    let (mut paid, mut got, mut exec, mut released, mut table) = (0u128, 0u128, 0u64, 0u128, 0u128);
    for k in 0..bets / 1_000 {
        c.jackpot_pool = 1_000 * JP_MIN_POT;
        c.mini_pool = 1_000 * MINI_MIN_POT;
        c.super_pool = 1_000 * SUPER_MIN_POT;
        c.prize_pool = 1_000_000 * SOL;
        c.seed_reserve = 0;
        p.balance = 1_000 * COST;
        p.jp_locked = 0;
        p.jp_per_week = 0;
        engine::commit(&mut c, &mut p, T0 + 1, k + 1, 1_000, false, key(9)).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(sm.seed()), T0 + 2).unwrap();
        assert_eq!(out.bank_released, out.spins * share(BANK_RELEASE_BPS));
        assert_eq!(c.seed_reserve, out.bank_released, "the pots are above their starting amounts: nothing of the release is used inside the round");
        paid += 1_000 * COST as u128;
        got += p.balance as u128 + p.jp_locked as u128;
        exec += out.spins;
        released += out.bank_released as u128;
        table += out.prizes as u128;
        p.balance = 0;
    }
    let o = OddsBase::new_mode(JP_MIN_POT, SUPER_MIN_POT, 0).at_super(u64::MAX / 4, u64::MAX / 4, u64::MAX / 4, true);
    let (jw, mw, sw) = (JP_MIN_POT * 3 / 100 * 76, MINI_MIN_POT * 3 - bps(MINI_MIN_POT * 3, MINI_KEEP_BPS), SUPER_MIN_POT / 2 * 5 / 10 * 9);
    // every pot win paid exactly the capped amount less the kept share, whatever the pot held
    assert_eq!(got, table + c.jackpots_won as u128 * jw as u128 + c.minis_won as u128 * mw as u128 + c.supers_won as u128 * sw as u128);
    let exact = spin_ev_q32(&o, 0, jw, mw, sw) as f64 / (Q32 - o.again as u128) as f64 / COST as f64;
    let rtp = got as f64 / paid as f64;
    let expect = |odds: u64| exec as f64 * odds as f64 / Q32 as f64;
    println!(
        "{bets} bets placed only in the richest state: returned {:.3}% (exact expectation {:.3}%); super jackpots {} (expected {:.0}), jackpots {} (expected {:.0}), minis {} (expected {:.0}); table prizes {:.3}% of the bets (expected {:.3}%); released to the reserve {:.3}% of the bets",
        rtp * 100.0, exact * 100.0, c.supers_won, expect(o.superj), c.jackpots_won, expect(o.jackpot), c.minis_won, expect(o.mini), table as f64 / paid as f64 * 100.0, mode_ev_ppm_tokens(0) as f64 / 1e6 / 0.884 / 100.0, released as f64 / paid as f64 * 100.0
    );
    assert!((rtp - exact).abs() < 0.009, "measured {rtp}, exact {exact}");
    for (won, odds) in [(c.supers_won, o.superj), (c.jackpots_won, o.jackpot), (c.minis_won, o.mini)] {
        let e = expect(odds);
        assert!((won as f64 - e).abs() < 4.0 * e.sqrt(), "pot wins {won}, expected {e}");
    }
    assert_eq!(released, exec as u128 * share(BANK_RELEASE_BPS) as u128);
}

// ---------------------------------------------------------------------------------------------------------------------------
// 4. Nobody can aim the release
// ---------------------------------------------------------------------------------------------------------------------------

#[test]
fn donations_and_administrator_moves_do_not_trigger_or_enlarge_a_release() {
    let lim = share(BANK_RELEASE_BPS);
    let run = |c: &mut Config, seed: [u8; 32]| {
        let mut p = player(c, 1);
        p.balance = 10 * COST;
        engine::commit(c, &mut p, T0 + 1, 1, 10, false, key(9)).unwrap();
        engine::settle(c, &mut p, Some(seed), T0 + 2).unwrap()
    };
    let seed = [9u8; 32];
    // a bank below the threshold: a donation of any size goes to the reserve and releases nothing
    let (mut c, _) = funded(BANK_KEEP - 100 * SOL, 5 * SOL, 20 * SOL, 0);
    engine::donate(&mut c, 10_000 * SOL).unwrap();
    assert_eq!((c.prize_pool, c.seed_reserve), (BANK_KEEP - 100 * SOL, 10_000 * SOL - mini_start(5 * SOL).min(0)));
    let out = run(&mut c, seed);
    assert_eq!(out.bank_released, 0);
    // the administrator's move (`rescale`): refused whenever it would leave the bank above the threshold, accepted up to it.
    // After it the bank holds exactly BANK_KEEP, so the settlement releases only what the round itself added to the bank.
    let both = c.prize_pool as u128 + c.seed_reserve as u128;
    let room = BANK_KEEP - c.prize_pool;
    assert!(room > 0 && room < c.seed_reserve);
    let before = (c.prize_pool, c.seed_reserve);
    assert!(engine::rescale(&mut c, 5_000 * SOL, 0, 0).is_err() && engine::rescale(&mut c, room + 1, 0, 0).is_err());
    assert_eq!((c.prize_pool, c.seed_reserve), before, "a refused move changes nothing");
    engine::rescale(&mut c, room, 0, 0).unwrap();
    assert_eq!((c.prize_pool, c.seed_reserve), (BANK_KEEP, before.1 - room));
    assert!(engine::rescale(&mut c, 1, 0, 0).is_err(), "a bank that holds BANK_KEEP takes nothing more from the reserve");
    let (mut c1, mut c2) = (c.clone(), c.clone());
    engine::donate(&mut c2, 77_777 * SOL).unwrap(); // somebody donates before the settlement
    let (o1, o2) = (run(&mut c1, seed), run(&mut c2, seed));
    assert!(o1.bank_released <= o1.spins * lim && o1.bank_released <= o1.spins * share(S_PRIZE), "at most what this round added to the bank");
    assert_eq!((o1.bank_released, o1.prizes, o1.spins), (o2.bank_released, o2.prizes, o2.spins), "a donation changes neither the release nor the round");
    assert!(c1.prize_pool <= BANK_KEEP + o1.spins * share(S_PRIZE) - o1.bank_released);
    assert!(c1.prize_pool as u128 + c1.seed_reserve as u128 <= both + o1.spins as u128 * share(S_PRIZE) as u128);
    // the pots cannot be fed by command: only a win tops a pot up from the reserve
    assert!(engine::rescale(&mut c, 0, 1, 0).is_err() && engine::rescale(&mut c, 0, 0, 1).is_err() && engine::rescale(&mut c, 1, 1, 0).is_err());
    // a refund releases nothing
    let (mut c, _) = funded(BANK_KEEP + 500 * SOL, 5 * SOL, 20 * SOL, 0);
    let mut p = player(&mut c, 2);
    p.balance = 1_000 * COST;
    engine::commit(&mut c, &mut p, T0 + 1, 1, 1_000, false, key(9)).unwrap();
    let r0 = (c.prize_pool, c.seed_reserve);
    engine::refund(&mut c, &mut p, T0 + 2).unwrap();
    assert_eq!((c.prize_pool, c.seed_reserve), r0);
}

/// The first application of the rules to a state left by the program deployed first (v1). An ordinary v1 bank is far below
/// BANK_KEEP: nothing moves. A bank that is far above it (here 5,000 SOL) is not emptied in one step: each settlement releases at
/// most 0.5% of a bet per executed spin, so the excess of 3,400 SOL leaves over ≈ 600,000 SOL of bets, visibly and evenly.
#[test]
fn first_application_to_a_v1_era_state() {
    // 1. an ordinary state: bank 200, reserve 30
    let (mut c, _) = funded(200 * SOL, 5 * SOL, 20 * SOL, 30 * SOL);
    let mut p = player(&mut c, 1);
    let mut sm = Sm(5);
    for k in 0..2_000u64 {
        p.balance = 100 * COST;
        engine::commit(&mut c, &mut p, T0 + 1, k + 1, 100, false, key(9)).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(sm.seed()), T0 + 2).unwrap();
        assert_eq!(out.bank_released, 0);
    }
    // 2. a bank far above the threshold
    let (mut c, _) = funded(5_000 * SOL, 5 * SOL, 20 * SOL, 30 * SOL);
    let mut p = player(&mut c, 1);
    let (mut released, mut exec, mut staked) = (0u128, 0u128, 0u128);
    let mut first = 0u64;
    let mut k = 0u64;
    let reserve0 = c.seed_reserve;
    let mut topups_seen = 0u128;
    while staked < 3_000_000 * COST as u128 {
        k += 1;
        let n = [10u16, 100, 1000][(k % 3) as usize];
        p.balance = n as u64 * COST;
        p.jp_locked = 0;
        p.jp_per_week = 0;
        engine::commit(&mut c, &mut p, T0 + 1, k, n, false, key(9)).unwrap();
        let (res_before, bank_before) = (c.seed_reserve, c.prize_pool);
        let out = engine::settle(&mut c, &mut p, Some(sm.seed()), T0 + 2).unwrap();
        assert!(out.bank_released <= out.spins * share(BANK_RELEASE_BPS));
        assert!(c.prize_pool >= BANK_KEEP && bank_before >= BANK_KEEP);
        // the reserve changes only by the release (in) and by top-ups after wins (out)
        assert!(c.seed_reserve <= res_before + out.bank_released);
        topups_seen += (res_before + out.bank_released - c.seed_reserve) as u128;
        if k == 1 {
            first = out.bank_released;
        }
        released += out.bank_released as u128;
        exec += out.spins as u128;
        staked += n as u128 * COST as u128;
    }
    assert!(first >= 100 * share(BANK_RELEASE_BPS) && first <= 216 * share(BANK_RELEASE_BPS) && first % share(BANK_RELEASE_BPS) == 0, "the first settlement (100 bets) releases 0.005–0.0108 SOL, not the excess of 3,400 SOL: {first}");
    assert_eq!(released, exec * share(BANK_RELEASE_BPS) as u128, "the limit binds all the way: the excess is not gone after 30,000 SOL of bets");
    assert_eq!(c.seed_reserve as u128, reserve0 as u128 + released - topups_seen);
    println!("v1-era bank of 5,000 SOL: after {:.0} SOL of bets {:.2} SOL released ({:.3}% of the bets), bank {:.1} SOL, reserve {:.2} SOL", staked as f64 / 1e9, released as f64 / 1e9, released as f64 / staked as f64 * 100.0, c.prize_pool as f64 / 1e9, c.seed_reserve as f64 / 1e9);
    assert!((released as f64 / staked as f64 - 0.005 / 0.884).abs() < 0.0001);
}

// ---------------------------------------------------------------------------------------------------------------------------
// 4b. The administrator cannot undo the release (re-verification of 18.11.2026, finding 1)
// ---------------------------------------------------------------------------------------------------------------------------

fn cfg_bytes(c: &Config) -> Vec<u8> {
    use anchor_lang::AccountSerialize;
    let mut d = Vec::new();
    c.try_serialize(&mut d).unwrap();
    d
}

/// `rescale` at the boundary: the bank may be brought up to BANK_KEEP and not a lamport above it.
#[test]
fn rescale_never_leaves_the_bank_above_bank_keep() {
    let at = |bank: u64, reserve: u64| {
        let (c, _) = funded(bank, 5 * SOL, 20 * SOL, reserve + mini_start(5 * SOL));
        assert_eq!((c.prize_pool, c.seed_reserve), (bank, reserve));
        c
    };
    // (bank, reserve, amount, accepted)
    let cases: [(u64, u64, u64, bool); 16] = [
        (100 * SOL, 500 * SOL, 500 * SOL, true),                       // launch-size bank, the whole reserve
        (100 * SOL, 5_000 * SOL, 1_500 * SOL, true),                   // exactly up to BANK_KEEP
        (100 * SOL, 5_000 * SOL, 1_500 * SOL + 1, false),              // one lamport above
        (100 * SOL, 5_000 * SOL, 5_000 * SOL, false),
        (0, BANK_KEEP, BANK_KEEP, true),
        (0, BANK_KEEP + 1, BANK_KEEP + 1, false),
        (BANK_KEEP - 1, 10 * SOL, 1, true),
        (BANK_KEEP - 1, 10 * SOL, 2, false),
        (BANK_KEEP, 10 * SOL, 1, false),                               // a bank at the threshold takes nothing
        (BANK_KEEP + 1, 10 * SOL, 1, false),                           // nor a bank above it: the release is not undone
        (BANK_KEEP + 3_400 * SOL, 10 * SOL, 10 * SOL, false),          // a v1-era bank
        (63_000 * SOL, 2_000 * SOL, 1, false),
        (100 * SOL, 10 * SOL, 10 * SOL + 1, false),                    // more than the reserve holds
        (100 * SOL, 10 * SOL, 0, false),
        (100 * SOL, 0, 1, false),
        (u64::MAX - 5, 10, 10, false),                                 // the sum does not fit: refused, no wrap-around
    ];
    for (bank, reserve, amount, accepted) in cases {
        let mut c = if bank > 1_000_000 * SOL { let mut c = at(0, reserve); c.prize_pool = bank; c } else { at(bank, reserve) };
        let before = cfg_bytes(&c);
        let r = engine::rescale(&mut c, amount, 0, 0);
        assert_eq!(r.is_ok(), accepted, "bank {bank}, reserve {reserve}, amount {amount}");
        if accepted {
            assert_eq!((c.prize_pool, c.seed_reserve), (bank + amount, reserve - amount));
            assert!(c.prize_pool <= BANK_KEEP);
            assert_eq!(bank_release(c.prize_pool, u64::MAX), 0, "right after a move there is nothing to release");
        } else {
            assert_eq!(cfg_bytes(&c), before, "a refused move changes nothing");
        }
        // the pots: never by command, whatever the bank holds (audit No. 3, S1)
        for (tp, tj, ts) in [(0, 1, 0), (0, 0, 1), (amount, 1, 0), (amount, 0, 1), (0, amount.max(1), amount.max(1)), (1, u64::MAX, 0), (1, 0, u64::MAX)] {
            let before = cfg_bytes(&c);
            assert!(engine::rescale(&mut c, tp, tj, ts).is_err());
            assert_eq!(cfg_bytes(&c), before);
        }
    }
    let mut unfunded = Config::default();
    unfunded.seed_reserve = SOL;
    assert!(engine::rescale(&mut unfunded, 1, 0, 0).is_err());
}

/// Property: on 400,000 random states and amounts (dense around BANK_KEEP and around the reserve) `rescale` is accepted
/// exactly when nothing goes to a pot, 0 < amount ≤ reserve and bank + amount ≤ BANK_KEEP; an accepted move changes only
/// the two pools, by the same amount; a refused one changes nothing. No command moves the reserve into a pot.
#[test]
fn rescale_property_accepted_exactly_up_to_bank_keep() {
    let mut sm = Sm(0xF1C5_0003);
    let (mut yes, mut no_keep, mut no_other) = (0u32, 0u32, 0u32);
    for k in 0..400_000u64 {
        let (mut c, _) = funded(200 * SOL, JP_MIN_POT + sm.below(30 * SOL), SUPER_MIN_POT + sm.below(300 * SOL), 40 * SOL);
        c.prize_pool = match sm.below(6) {
            0 => sm.below(BANK_KEEP + 1),
            1 => BANK_KEEP - sm.below(3 * SOL).min(BANK_KEEP),
            2 => BANK_KEEP + sm.below(3 * SOL),
            3 => BANK_KEEP - sm.below(1_000).min(BANK_KEEP) + sm.below(1_000),
            4 => sm.below(70_000 * SOL),
            _ => sm.next() >> sm.below(40),
        };
        c.seed_reserve = match sm.below(4) {
            0 => sm.below(5 * SOL),
            1 => sm.below(4_000 * SOL),
            2 => BANK_KEEP.saturating_sub(c.prize_pool) + sm.below(3) - sm.below(3).min(BANK_KEEP.saturating_sub(c.prize_pool)),
            _ => sm.next() >> sm.below(40),
        };
        c.jackpot_pool = sm.below(20 * SOL);
        c.mini_pool = sm.below(2 * SOL);
        c.super_pool = sm.below(900 * SOL);
        let room = BANK_KEEP.saturating_sub(c.prize_pool);
        let amount = match sm.below(7) {
            0 => room,
            1 => room.saturating_add(1),
            2 => room.saturating_sub(1),
            3 => c.seed_reserve,
            4 => c.seed_reserve.saturating_add(1),
            5 => sm.below(c.seed_reserve.max(1) + 2),
            _ => sm.next() >> sm.below(64),
        };
        let (tj, ts) = if k % 9 == 0 { (sm.below(2) * (1 + sm.below(SOL)), sm.below(2) * (1 + sm.below(SOL))) } else { (0, 0) };
        let (bank, reserve) = (c.prize_pool, c.seed_reserve);
        let before = c.clone();
        let expected = tj == 0 && ts == 0 && amount > 0 && amount <= reserve && bank as u128 + amount as u128 <= BANK_KEEP as u128;
        let r = engine::rescale(&mut c, amount, tj, ts);
        assert_eq!(r.is_ok(), expected, "bank {bank}, reserve {reserve}, amount {amount}, pots {tj} {ts}");
        if expected {
            yes += 1;
            assert!(c.prize_pool <= BANK_KEEP && bank < BANK_KEEP);
            assert_eq!((c.prize_pool, c.seed_reserve), (bank + amount, reserve - amount));
            // nothing else moved: put the two pools back and the account is byte-identical
            let mut back = c.clone();
            back.prize_pool = bank;
            back.seed_reserve = reserve;
            assert_eq!(cfg_bytes(&back), cfg_bytes(&before));
            assert_eq!(pools(&c), pools(&before));
        } else {
            if tj == 0 && ts == 0 && amount > 0 && amount <= reserve {
                no_keep += 1;
            } else {
                no_other += 1;
            }
            assert_eq!(cfg_bytes(&c), cfg_bytes(&before));
        }
        assert_eq!((c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_pool), (before.jackpot_pool, before.mini_pool, before.super_pool, before.seed_pool));
    }
    println!("rescale on 400,000 random states: {yes} accepted, {no_keep} refused only because the bank would end above BANK_KEEP, {no_other} refused for the amount or a pot");
    assert!(yes > 20_000 && no_keep > 20_000 && no_other > 20_000);
}

/// The attack of the re-verification, finding 1: after every settlement the administrator moves back into the bank as much
/// of the reserve as the program accepts. Three runs of the same bets (wheel, rounds of 10…1000, bank 40 SOL above BANK_KEEP,
/// empty reserve):
///   * honest: the administrator does nothing;
///   * attack: the largest accepted move after every settlement, and the whole reserve tried first;
///   * control: the program as it was BEFORE the fix (the move applied to the fields directly, no bound).
/// Exact, at every step of the attack run: a move is never accepted while the bank holds BANK_KEEP or more; and all the
/// administrator ever moved is at most what the bank had lost to players below BANK_KEEP (u = min(bank, BANK_KEEP) rises
/// only by moves and by the bank's own gain, and never exceeds BANK_KEEP). So the reserve returns to the bank only in place
/// of prizes the bank has paid, never in place of the surplus. The control shows what the bound prevents: no pot is ever
/// topped up and the bank keeps everything.
#[test]
fn the_administrator_cannot_cancel_the_release() {
    let stakes: u64 = std::env::var("FD_RESCALE_STAKES").ok().and_then(|s| s.parse().ok()).unwrap_or(6_000_000);
    #[derive(Clone, Copy, PartialEq)]
    enum Admin {
        Honest,
        Attack,
        OldProgram,
    }
    struct Res {
        bank_end: u64,
        bank_max: u64,
        reserve_end: u64,
        released: u128,
        moved: u128,
        topups: u128,
        refused_above: u64,
        accepted: u64,
        staked: u128,
        returned: u128,
    }
    let run = |admin: Admin, bank0: u64, reserve0: u64, seed: u64| -> Res {
        let (mut c, _) = funded(bank0, 5 * SOL, 80 * SOL, reserve0 + mini_start(5 * SOL));
        assert_eq!((c.prize_pool, c.seed_reserve), (bank0, reserve0));
        let mut p = player(&mut c, 1);
        let mut sm = Sm(seed);
        let (mut released, mut moved, mut topups, mut staked, mut returned) = (0u128, 0u128, 0u128, 0u128, 0u128);
        let (mut refused_above, mut accepted, mut bank_max) = (0u64, 0u64, 0u64);
        // u = min(bank, BANK_KEEP); lost = what settlements took from u
        let u_of = |c: &Config| c.prize_pool.min(BANK_KEEP) as u128;
        let u0 = u_of(&c);
        let mut lost = 0u128;
        let mut k = 0u64;
        while staked < stakes as u128 * COST as u128 {
            k += 1;
            let n = [10u16, 100, 1000, 100][(k % 4) as usize];
            p.balance = n as u64 * COST;
            p.jp_locked = 0;
            p.jp_per_week = 0;
            engine::commit(&mut c, &mut p, T0 + 1, k, n, false, key(9)).unwrap();
            let (u_before, res_before) = (u_of(&c), c.seed_reserve);
            let out = engine::settle(&mut c, &mut p, Some(sm.seed()), T0 + 2).unwrap();
            returned += p.balance as u128 + p.jp_locked as u128;
            staked += n as u128 * COST as u128;
            released += out.bank_released as u128;
            topups += (res_before + out.bank_released - c.seed_reserve) as u128;
            lost += u_before.saturating_sub(u_of(&c));
            bank_max = bank_max.max(c.prize_pool);
            match admin {
                Admin::Honest => {}
                Admin::OldProgram => {
                    // the function before the fix: any amount up to the reserve
                    let a = c.seed_reserve;
                    c.seed_reserve -= a;
                    c.prize_pool += a;
                    moved += a as u128;
                }
                Admin::Attack => {
                    let (bank, reserve) = (c.prize_pool, c.seed_reserve);
                    if reserve > 0 {
                        // the verifier's move: everything back
                        let all = engine::rescale(&mut c, reserve, 0, 0);
                        if bank >= BANK_KEEP || reserve > BANK_KEEP - bank {
                            assert!(all.is_err());
                            assert_eq!((c.prize_pool, c.seed_reserve), (bank, reserve));
                            if bank >= BANK_KEEP {
                                refused_above += 1;
                                assert!(engine::rescale(&mut c, 1, 0, 0).is_err(), "not one lamport while the bank holds BANK_KEEP or more");
                            } else {
                                // the most the program accepts
                                let room = BANK_KEEP - bank;
                                assert!(engine::rescale(&mut c, room + 1, 0, 0).is_err());
                                engine::rescale(&mut c, room, 0, 0).unwrap();
                                moved += room as u128;
                                accepted += 1;
                            }
                        } else {
                            all.unwrap();
                            moved += reserve as u128;
                            accepted += 1;
                        }
                        assert!(c.prize_pool <= BANK_KEEP.max(bank));
                    }
                    // exact: the administrator moved at most the bank's initial shortfall plus what the bank lost below BANK_KEEP
                    assert!(moved <= BANK_KEEP as u128 - u0 + lost, "moved {moved}, lost below the threshold {lost}");
                }
            }
        }
        Res { bank_end: c.prize_pool, bank_max, reserve_end: c.seed_reserve, released, moved, topups, refused_above, accepted, staked, returned }
    };
    let sol = |x: u128| x as f64 / 1e9;
    // scenario 1: the bank starts 40 SOL above BANK_KEEP with an empty reserve (a v1-era surplus);
    // scenario 2: the bank starts 50 SOL below it and the reserve holds 200 SOL (the function's legitimate use, then the attack)
    for (title, bank0, reserve0, seed) in [("bank 40 SOL above BANK_KEEP, empty reserve", BANK_KEEP + 40 * SOL, 0u64, 0xBA4C_2026u64), ("bank 50 SOL below BANK_KEEP, reserve 200 SOL", BANK_KEEP - 50 * SOL, 200 * SOL, 0xBA4C_2027)] {
        let (h, a, o) = (run(Admin::Honest, bank0, reserve0, seed), run(Admin::Attack, bank0, reserve0, seed), run(Admin::OldProgram, bank0, reserve0, seed));
        println!("{title}:");
        for (name, r) in [("honest", &h), ("attack (largest accepted move after every settlement)", &a), ("control: the program before the fix", &o)] {
            println!(
                "  {name}: {:.0} SOL of bets; bank at the end {:.1} SOL (largest {:.1}); released {:.2} SOL, moved back {:.2} SOL ({} moves accepted, {} refused with the bank at or above BANK_KEEP); pots topped up with {:.2} SOL; reserve at the end {:.2} SOL; returned to the player {:.3}%",
                sol(r.staked), sol(r.bank_end as u128), sol(r.bank_max as u128), sol(r.released), sol(r.moved), r.accepted, r.refused_above, sol(r.topups), sol(r.reserve_end as u128), r.returned as f64 / r.staked as f64 * 100.0
            );
        }
        // the ledger of the reserve in every run
        for r in [&h, &a, &o] {
            assert_eq!(r.topups + r.reserve_end as u128 + r.moved, reserve0 as u128 + r.released);
        }
        // the control reproduces the finding: the reserve is always empty when a pot is won, so nothing ever reaches a pot,
        // and the bank keeps the surplus (and takes the donated reserve as well)
        assert_eq!((o.topups, o.reserve_end as u128), (0, 0));
        assert_eq!(o.moved, o.released + reserve0 as u128);
        // with the fix the attack cannot do that: moves are refused above the threshold, the surplus leaves the bank and the pots are topped up
        assert!(a.refused_above > 0 && a.topups > 0 && h.topups > 0 && h.moved == 0);
        assert!(a.moved <= (BANK_KEEP.saturating_sub(bank0)) as u128 + a.released, "the attack never moves back more than the shortfall it filled");
        assert!(a.topups * 2 >= h.topups, "attack {} vs honest {}", a.topups, h.topups);
        assert!(o.bank_end as u128 > a.bank_end as u128 + a.topups / 2, "before the fix the bank kept what the pots receive now");
        if bank0 < BANK_KEEP {
            assert!(a.accepted > 0, "the legitimate use still works: a bank below BANK_KEEP is brought up to it");
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------
// 5. Long simulation: the ledger exact after every operation
// ---------------------------------------------------------------------------------------------------------------------------

/// 20 million paid bets (200,000 SOL) of 40 players, every game, rounds of 10…1000 bets, deposits, withdrawals, weekly
/// instalments, late settlements and refunds; the administrator changes the amounts within the allowed bounds, collects the
/// income and NEVER donates; the reserve starts EMPTY and the bank starts 60 SOL below BANK_KEEP, so both phases are covered:
/// pots restarting on their own, then the bank's surplus arriving. After every operation: vault = pools + obligations.
#[test]
fn long_simulation_both_rules_ledger_exact_every_step() {
    let target: u64 = std::env::var("FD_SELF_STAKES").ok().and_then(|s| s.parse().ok()).unwrap_or(20_000_000);
    let (mut c, mut vault) = funded(BANK_KEEP - 60 * SOL, 5 * SOL, 80 * SOL, 0);
    let funded_total = vault as u128;
    let mut sm = Sm(0x5E1F_2026_1118);
    let np = 40usize;
    let mut ps: Vec<Player> = (0..np as u64).map(|i| player(&mut c, i)).collect();
    let mut owed_total: u128 = 0;
    let (mut staked, mut exec, mut deposited, mut withdrawn, mut claimed, mut released, mut topups) = (0u128, 0u128, 0u128, 0u128, 0u128, 0u128, 0u128);
    let (mut rounds, mut refunds, mut lates, mut paid_stakes) = (0u64, 0u64, 0u64, 0u64);
    let (mut closed_j, mut closed_m, mut spins_seen) = (0u128, 0u128, 0u128);
    let (mut restarts_j, mut restarts_m): (Vec<f64>, Vec<f64>) = (vec![], vec![]);
    let (mut first_release, mut bank_min, mut reserve_max) = (None, u64::MAX, 0u64);
    let mut clock = (T0 + 10) as f64;
    let secs = 500.0 * 86_400.0 / (target as f64 / 110.0);
    while paid_stakes < target {
        rounds += 1;
        clock += secs;
        let now = clock as i64;
        if rounds % 5_000 == 0 && c.dev_pool > 0 {
            claimed += c.dev_pool as u128;
            vault -= c.dev_pool;
            c.dev_pool = 0;
        }
        if rounds % 40_000 == 0 {
            // the administrator moves the jackpot amount between 5 and 8 SOL when the bounds allow it (rejected otherwise)
            let j = if engine::jackpot_start_of(&c) == 5 * SOL { 8 * SOL } else { 5 * SOL };
            let s_now = engine::super_start_of(&c);
            let _ = engine::set_starts(&mut c, j, s_now, now);
        }
        let i = sm.below(np as u64) as usize;
        let p = &mut ps[i];
        let b = owed(p);
        let mode = if sm.below(3) == 0 { 0 } else { sm.below(MODES as u64) as u8 };
        let n = if mode == 0 { [10u16, 10, 20, 50, 100, 100, 250, 500, 1000][sm.below(9) as usize] } else { [10u16, 10, 20, 25, 50, 100][sm.below(6) as usize] };
        let stake = n as u64 * COST;
        if p.balance < stake {
            let d = stake * (1 + sm.below(20));
            engine::deposit(p, d, now).unwrap();
            vault += d;
            deposited += d as u128;
        }
        engine::commit_mode(&mut c, p, now, rounds, n, mode, false, key(9)).unwrap();
        let fate = sm.below(1_000);
        if fate == 0 {
            engine::refund(&mut c, p, now + 600).unwrap();
            refunds += 1;
        } else {
            let (jw, mw) = (c.jackpots_won, c.minis_won);
            let (res0, bank0) = (c.seed_reserve, c.prize_pool);
            let (js, ms) = (jackpot_start(p.p_start_jackpot), mini_start(p.p_start_jackpot));
            let (cj, cm) = (c.jackpot_pool < open_pot(js), c.mini_pool < open_pot(ms));
            let out = if fate < 12 {
                lates += 1;
                engine::settle_late(&mut c, p, sm.seed(), now + 1_300).unwrap()
            } else {
                engine::settle(&mut c, p, Some(sm.seed()), now + 2).unwrap()
            };
            assert!(out.bank_released <= out.spins * share(BANK_RELEASE_BPS));
            assert!(out.bank_released == 0 || c.prize_pool >= BANK_KEEP);
            assert!(c.seed_reserve <= res0 + out.bank_released, "the reserve receives only the release (nobody donates here)");
            topups += (res0 + out.bank_released - c.seed_reserve) as u128;
            released += out.bank_released as u128;
            if out.bank_released > 0 && first_release.is_none() {
                first_release = Some((paid_stakes, bank0));
            }
            spins_seen += out.spins as u128;
            closed_j += if cj { out.spins as u128 } else { 0 };
            closed_m += if cm { out.spins as u128 } else { 0 };
            // a single win in the round: what the pot restarted from (its own contributions after the win are below 3% of a start)
            if c.jackpots_won == jw + 1 && out.jp_hits == 1 {
                restarts_j.push(c.jackpot_pool as f64 / engine::jackpot_start_of(&c) as f64);
            }
            if c.minis_won == mw + 1 && out.mini_hits == 1 && out.jp_hits == 0 {
                restarts_m.push(c.mini_pool as f64 / mini_start(c.init_jackpot) as f64);
            }
            staked += stake as u128;
            exec += out.spins as u128;
            paid_stakes += n as u64;
        }
        if sm.below(4) == 0 {
            let w = engine::withdrawable(p, now + 2_000);
            if w > 0 {
                let (net, fee, _) = engine::withdraw(&mut c, p, w, now + 2_000).unwrap();
                assert_eq!(fee, 0);
                vault -= net;
                withdrawn += net as u128;
            }
        }
        owed_total = owed_total + owed(p) - b;
        assert_eq!(vault as u128, pools(&c) + owed_total, "round {rounds}");
        bank_min = bank_min.min(c.prize_pool);
        reserve_max = reserve_max.max(c.seed_reserve);
    }
    assert_eq!(owed_total, ps.iter().map(owed).sum::<u128>());
    assert_eq!(c.dev_total as u128, exec * share(S_DEV) as u128);
    assert_eq!(c.dev_pool as u128 + claimed, c.dev_total as u128);
    assert_eq!(c.promo_pool, 0);
    assert_eq!(funded_total + deposited, withdrawn + claimed + pools(&c) + owed_total);
    assert_eq!(c.seed_reserve as u128, released - topups, "the reserve holds what was released and not yet used");
    let dev_pct = c.dev_total as f64 / staked as f64 * 100.0;
    let rtp = c.paid_total as f64 / staked as f64 * 100.0;
    let stat = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (v.len(), v.iter().sum::<f64>() / v.len().max(1) as f64, v[v.len() / 2], v.iter().filter(|x| **x < 0.25).count() as f64 / v.len().max(1) as f64 * 100.0)
    };
    let (sj, sm_) = (stat(&mut restarts_j), stat(&mut restarts_m));
    println!("{rounds} rounds, {paid_stakes} paid bets ({:.0} SOL), {refunds} refunds, {lates} late: vault = pools + obligations after every operation", staked as f64 / 1e9);
    println!("administrator {dev_pct:.4}% of paid bets; credited to players {rtp:.3}%; released from the bank {:.2} SOL ({:.4}% of paid bets), used by top-ups {:.2} SOL; first release after {:?} paid bets", released as f64 / 1e9, released as f64 / staked as f64 * 100.0, topups as f64 / 1e9, first_release.map(|x| x.0));
    println!("jackpot: {} restarts, mean {:.2} / median {:.2} starting amounts, below a quarter {:.1}%; closed {:.2}% of spins", sj.0, sj.1, sj.2, sj.3, closed_j as f64 / spins_seen as f64 * 100.0);
    println!("mini:    {} restarts, mean {:.2} / median {:.2} starting amounts, below a quarter {:.1}%; closed {:.2}% of spins", sm_.0, sm_.1, sm_.2, sm_.3, closed_m as f64 / spins_seen as f64 * 100.0);
    println!("bank now {:.1} SOL (lowest {:.1}); reserve now {:.2} SOL (highest {:.2})", c.prize_pool as f64 / 1e9, bank_min as f64 / 1e9, c.seed_reserve as f64 / 1e9, reserve_max as f64 / 1e9);
    if target >= 20_000_000 {
        assert!((dev_pct - 6.991).abs() < 0.01);
        assert!(rtp > 90.0 && rtp < 94.5, "{rtp}");
        assert!(sj.0 > 200 && sm_.0 > 1_000);
        // against the rules before this change with an empty reserve (8% and 19% of spins closed, 66% and 94% of restarts below a quarter)
        assert!((closed_j as f64) < 0.04 * spins_seen as f64 && (closed_m as f64) < 0.04 * spins_seen as f64);
        assert!(sj.3 < 35.0 && sm_.3 < 30.0 && sj.2 > 0.35 && sm_.2 > 0.35);
    }
}
