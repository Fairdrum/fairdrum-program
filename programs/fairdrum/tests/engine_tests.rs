//! Tests of the pure game logic (no Solana), economy of upgrade No. 2: 6.99% of paid bets to the administrator, 5.01% to the jackpots
//! (3.31% of them to the super jackpot), the rest is prizes. The economics of the change itself: tests/econ_tests.rs.
//! Run: `cargo test -p fairdrum --release -- --nocapture`. Tests of the previous economy are in tests/_v1 (not compiled).

use anchor_lang::prelude::Pubkey;
use fairdrum::consts::*;
use fairdrum::engine;
use fairdrum::math::*;
use fairdrum::state::*;

const T0: i64 = 1_800_000_000;
const DAY: i64 = DAY_SECS;
const SOL: u64 = 1_000_000 * ONE;

// ---------- helpers ----------
struct Sm(u64);
impl Sm {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn seed(&mut self) -> [u8; 32] {
        let mut s = [0u8; 32];
        for i in 0..4 {
            s[i * 8..i * 8 + 8].copy_from_slice(&self.next().to_le_bytes());
        }
        s
    }
}

/// The bank as the administrator will open it: prize bank, starting jackpot and super, seed reserve
fn cfg_with(prize: u64, jackpot: u64, superj: u64, reserve: u64) -> (Config, u64) {
    let mut c = Config::default();
    c.start_ts = T0;
    c.init_prize = prize;
    c.prize_buffer = 1_000_000 * SOL;
    c.init_seed_reserve = reserve;
    c.init_jackpot = jackpot;
    c.init_super = superj;
    let vault = prize + jackpot + superj + reserve;
    engine::fund(&mut c, vault).unwrap();
    (c, vault)
}
fn funded_cfg() -> Config {
    cfg_with(50 * SOL, 5 * SOL, 20 * SOL, 100 * SOL).0
}
fn key(i: u64) -> Pubkey {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&i.to_le_bytes());
    b[31] = 7;
    Pubkey::new_from_array(b)
}
fn new_player(c: &mut Config, i: u64, now: i64) -> Player {
    let n = engine::assign_number(c).unwrap();
    let mut p = Player::default();
    engine::init_player(&mut p, key(i), 255, n, now);
    p
}
fn ledger(c: &Config, players: &[&Player]) -> u128 {
    let pools = c.act_pool as u128 + c.prize_pool as u128 + c.jackpot_pool as u128 + c.mini_pool as u128 + c.super_pool as u128 + c.seed_pool as u128
        + c.seed_reserve as u128 + c.dev_pool as u128 + c.promo_pool as u128;
    let pl: u128 = players.iter().map(|p| p.balance as u128 + p.jp_locked as u128 + p.p_stake as u128).sum();
    pools + pl
}
fn q(p: u64) -> f64 {
    p as f64 / 4_294_967_296.0
}

// ---------- bet shares ----------
#[test]
fn split_is_seven_percent_admin_five_percent_jackpots_rest_prizes() {
    assert_eq!(S_PRIZE + S_JACK + S_MINI + S_SUPER + S_SEED + S_DEV + S_BURN + S_ACT, BPS);
    assert_eq!(S_BURN + S_ACT, 0);
    // in terms of paid bets: there are 1/(1 − "Again") times more executed spins than paid ones
    let per_paid = 1.0 / (1.0 - W_AGAIN as f64 / BPS as f64);
    let admin = S_DEV as f64 / BPS as f64 * per_paid;
    let jackpots = (S_JACK + S_MINI + S_SUPER + S_SEED) as f64 / BPS as f64 * per_paid;
    assert!(admin <= 0.07 && admin > 0.0699, "administrator {admin}: at most 7% of paid bets");
    assert!((jackpots - 0.0501).abs() < 0.0001, "to the jackpots {jackpots}");
    assert_eq!((S_DEV, S_SUPER, S_PRIZE, S_JACK, S_MINI, S_SEED), (618, 293, 8_939, 88, 44, 18));
    assert_eq!(COST, 10_000_000, "bet of 0.01 SOL in lamports");
    assert_eq!(share(S_PRIZE) + share(S_JACK) + share(S_MINI) + share(S_SUPER) + share(S_SEED) + share(S_DEV), COST);
}

// ---------- jackpots: chance = contribution ÷ the administrator's starting amount ----------
#[test]
fn jackpot_odds_are_contribution_over_admin_start_amount() {
    for (j0, s0) in [(SOL, 5 * SOL), (5 * SOL, 20 * SOL), (50 * SOL, 500 * SOL)] {
        let ob = OddsBase::new_mode(j0, s0, 0);
        let o = ob.at_super(j0, j0 / MINI_START_DIV, s0, true);
        assert!((q(o.jackpot) * j0 as f64 / share(S_JACK) as f64 - 1.0).abs() < 0.002, "jackpot chance = contribution ÷ starting amount");
        assert!((q(o.mini) * (j0 / MINI_START_DIV) as f64 / share(S_MINI) as f64 - 1.0).abs() < 0.002);
        assert!((q(o.superj) * s0 as f64 / share(S_SUPER) as f64 - 1.0).abs() < 0.01);
        // the chance does not depend on how much the pot holds now (if it is open)
        assert_eq!(ob.at_super(40 * j0, 40 * j0, 40 * s0, true), o);
        // a pot is open from a quarter of the starting amount; a free repeat does not play the super
        let low = ob.at_super(j0 / 4 - 1, j0 / MINI_START_DIV / 4 - 1, s0 / 4 - 1, true);
        assert_eq!((low.jackpot, low.mini, low.superj), (0, 0, 0));
        assert_eq!(ob.at_super(j0, j0, s0, false).superj, 0);
    }
    // a 5 SOL jackpot is hard to win: 1 chance in roughly 57,000 spins
    let one_in = 1.0 / q(OddsBase::new_mode(5 * SOL, 0, 0).at_super(5 * SOL, 0, 0, true).jackpot);
    assert!(one_in > 56_000.0 && one_in < 57_500.0, "1 in {one_in}");
    // if the administrator has not set the amounts, the smallest starting ones apply
    let d = OddsBase::new_mode(0, 0, 0).at_super(JP_MIN_POT, MINI_MIN_POT, SUPER_MIN_POT, true);
    assert!((q(d.jackpot) * JP_MIN_POT as f64 / share(S_JACK) as f64 - 1.0).abs() < 0.002 && d.mini > 0 && d.superj > 0);
}

#[test]
fn main_draw_frequencies_match_tables() {
    for mode in 0..MODES as u8 {
        let ob = OddsBase::new_mode(5 * SOL, 20 * SOL, mode);
        let o = ob.at_super(0, 0, 0, true);
        let mut rng = Sm(100 + mode as u64);
        let (mut gold, mut again) = (0u64, 0u64);
        let n = 400_000;
        for _ in 0..n {
            match draw_main(rng.next() as u32, &o) {
                Main::Gold => gold += 1,
                Main::Again => again += 1,
                Main::Empty => {}
            }
        }
        let want = MODE_PPM[mode as usize] as f64 / 1e6;
        let got = gold as f64 / n as f64;
        assert!((got - want).abs() < 0.004, "mode {mode}: prize {got} instead of {want}");
        assert!((again as f64 / n as f64 - 0.116).abs() < 0.003);
    }
}

#[test]
fn mode_prizes_come_from_their_ladder_and_evs_are_equal() {
    let ev = |m: usize| MODE_PPM[m] as f64 / 1e6 * MODE_LADDER[m].iter().map(|(t, w)| *t as f64 * *w as f64).sum::<f64>() / 1e4;
    let ev0 = ev(0);
    // regular prizes return 77.5% of every spin; together with the "Again" repeat this is no more than the prize bank's share
    assert!((ev0 / 10_000.0 - 0.775).abs() < 0.0005, "{ev0}");
    assert!(ev0 / 10_000.0 + W_AGAIN as f64 / BPS as f64 <= S_PRIZE as f64 / BPS as f64);
    for m in 0..MODES {
        assert!(ev(m) <= ev0 + 1e-6 && ev(m) > ev0 * 0.995, "mode {m}: {}", ev(m));
        assert_eq!(MODE_LADDER[m].iter().map(|x| x.1).sum::<u64>(), LADDER_SUM);
        let o = OddsBase::new_mode(0, 0, m as u8).at_super(0, 0, 0, true);
        let mut rng = Sm(m as u64);
        for _ in 0..20_000 {
            match draw_gold_mode(rng.next(), rng.next() as u32, &o, m as u8) {
                Gold::Prize(t) => assert!(MODE_LADDER[m].iter().any(|(x, w)| *x == t && *w > 0), "mode {m}: prize {t} is not in the table"),
                other => panic!("with empty pots only a table prize is possible: {other:?}"),
            }
        }
    }
}

#[test]
fn rng_is_deterministic_and_seed_dependent() {
    let (mut a, mut b, mut c) = (Rng::new([1; 32]), Rng::new([1; 32]), Rng::new([2; 32]));
    let (x, y, z): (Vec<u32>, Vec<u32>, Vec<u32>) = ((0..50).map(|_| a.next_u32()).collect(), (0..50).map(|_| b.next_u32()).collect(), (0..50).map(|_| c.next_u32()).collect());
    assert_eq!(x, y);
    assert_ne!(x, z);
    assert_eq!(a.gold(7), b.gold(7));
    assert_ne!(a.gold(7), a.gold(8));
}

// ---------- launch, the administrator's starting amounts ----------
#[test]
fn funding_sets_admin_start_amounts() {
    let (c, _) = cfg_with(50 * SOL, 5 * SOL, 20 * SOL, 100 * SOL);
    assert_eq!((c.prize_pool, c.jackpot_pool, c.super_pool), (50 * SOL, 5 * SOL, 20 * SOL));
    assert_eq!(c.mini_pool, 5 * SOL / MINI_START_DIV, "the mini starts from a tenth of the starting jackpot");
    assert_eq!(c.seed_reserve, 100 * SOL - c.mini_pool);
    // an underfunded vault and a repeated opening are rejected
    let mut d = Config::default();
    d.init_jackpot = SOL;
    assert!(engine::fund(&mut d, SOL - 1).is_err());
    let mut c2 = c.clone();
    assert!(engine::fund(&mut c2, 1_000 * SOL).is_err());
    // the bank can also be opened empty
    let mut e = Config::default();
    engine::fund(&mut e, 0).unwrap();
    assert_eq!(e.jackpot_pool + e.mini_pool + e.super_pool + e.prize_pool, 0);
}

#[test]
fn donate_and_rescale_only_add_money_for_players() {
    let mut c = funded_cfg();
    let before = c.seed_reserve;
    engine::donate(&mut c, 7 * SOL).unwrap();
    assert_eq!(c.seed_reserve, before + 7 * SOL);
    assert!(engine::donate(&mut c, 0).is_err());
    // moving into the pots — only from the reserve and no more than it holds
    let total = ledger(&c, &[]);
    let (j0, s0) = (c.jackpot_pool, c.super_pool);
    engine::rescale(&mut c, SOL, 0, 0).unwrap();
    assert_eq!(ledger(&c, &[]), total, "moving into the prize bank neither creates nor destroys funds");
    // audit No. 3, S1: the reserve cannot be moved into the jackpot or the super jackpot — otherwise the administrator, seeing a jackpot
    // in their own unfinished round, would pour the reserve into the pot and take it
    assert!(engine::rescale(&mut c, 0, SOL, 0).is_err() && engine::rescale(&mut c, 0, 0, SOL).is_err() && engine::rescale(&mut c, SOL, 1, 0).is_err());
    assert_eq!((c.jackpot_pool, c.super_pool), (j0, s0));
    let too_much = c.seed_reserve + 1;
    assert!(engine::rescale(&mut c, too_much, 0, 0).is_err());
    assert!(engine::rescale(&mut c, 0, 0, 0).is_err());
    let mut unfunded = Config::default();
    assert!(engine::donate(&mut unfunded, SOL).is_err());
}

// ---------- deposit and withdrawal: no lock since upgrade No. 2 (the deployed v1 has 12 hours) ----------
#[test]
fn withdraw_is_open_at_once_after_a_deposit_and_is_free() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    assert_eq!(DEPOSIT_LOCK_SECS, 0);
    engine::deposit(&mut p, SOL, T0 + 10).unwrap();
    assert!(engine::deposit(&mut p, 0, T0 + 10).is_err());
    assert_eq!(engine::withdrawable(&p, T0 + 10), SOL, "the deposit can be withdrawn in the same second");
    assert!(engine::withdraw_open_ts(&p) <= T0 + 10, "the opening time shown to the site is not in the future");
    let (net, fee, burn) = engine::withdraw(&mut c, &mut p, SOL / 2, T0 + 10).unwrap();
    assert_eq!((net, fee, burn), (SOL / 2, 0, 0), "withdrawal without a project fee");
    assert_eq!(p.dep_locked, 0);
    engine::deposit(&mut p, SOL, T0 + 20 * 3_600).unwrap();
    assert_eq!(engine::withdrawable(&p, T0 + 20 * 3_600), SOL + SOL / 2);
    let all = p.balance;
    assert!(engine::withdraw(&mut c, &mut p, all + 1, T0 + 20 * 3_600).is_err(), "more than the balance cannot be withdrawn");
    assert!(engine::withdraw(&mut c, &mut p, all, T0 + 20 * 3_600).is_ok());
    assert_eq!((p.balance, p.dep_locked), (0, 0));
    assert!(engine::withdraw(&mut c, &mut p, 1, T0 + 33 * 3_600).is_err(), "more than the balance cannot be withdrawn");
    // a player who made no deposit (for example, one only receiving jackpot parts) withdraws without waiting
    let mut w = new_player(&mut c, 2, T0);
    w.balance = SOL;
    assert!(engine::withdraw(&mut c, &mut w, SOL, T0 + 1).is_ok());
}

// ---------- bets ----------
#[test]
fn commit_validations() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    engine::deposit(&mut p, 2_000 * COST, T0).unwrap();
    assert!(engine::commit(&mut c, &mut p, T0 - 1, 1, 1, false, key(9)).is_err(), "not allowed before the start");
    assert!(engine::commit(&mut c, &mut p, T0, 1, 0, false, key(9)).is_err(), "empty batch");
    assert!(engine::commit(&mut c, &mut p, T0, 1, BATCH_LIMIT + 1, false, key(9)).is_err(), "batch above the limit");
    assert!(engine::commit_mode(&mut c, &mut p, T0, 1, ARCADE_BATCH_LIMIT + 1, 3, false, key(9)).is_err(), "in the games a round is up to ×100");
    assert!(engine::commit_mode(&mut c, &mut p, T0, 1, 1, MODES as u8, false, key(9)).is_err(), "unknown mode");
    assert!(engine::commit(&mut c, &mut p, T0, 1, 1, true, key(9)).is_err(), "there are no free spins");
    let mut poor = new_player(&mut c, 2, T0);
    assert!(engine::commit(&mut c, &mut poor, T0, 1, 1, false, key(9)).is_err(), "not allowed without funds");
    engine::commit_mode(&mut c, &mut p, T0, 5, 100, 4, false, key(9)).unwrap();
    assert_eq!((p.balance, p.p_stake, pending_n(p.p_n), pending_mode(p.p_n)), (1_900 * COST, 100 * COST, 100, 4));
    // popularity counter: one round of game 4 — plus one; the round size and rejected attempts are not counted
    assert_eq!(c.mode_rounds[4], 1);
    assert_eq!(c.mode_rounds.iter().sum::<u64>(), 1, "rejected bets did not get into the counter");
    assert!(engine::commit(&mut c, &mut p, T0, 6, 1, false, key(9)).is_err(), "a new round cannot start until the current one is settled");
    let mut u = Config::default();
    assert!(engine::commit(&mut u, &mut p, T0, 1, 1, false, key(9)).is_err(), "the bank is not open");
    for n in [1u16, 7, 100, 1000] {
        for m in 0..MODES as u8 {
            assert_eq!((pending_n(pack_pending(n, m)), pending_mode(pack_pending(n, m))), (n, m));
        }
    }
}

#[test]
fn forfeit_keeps_stake_in_pools_and_is_exact_for_huge_batch() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    engine::deposit(&mut p, 1_000 * COST, T0).unwrap();
    let before = ledger(&c, &[&p]);
    engine::commit(&mut c, &mut p, T0, 1, 1_000, false, key(9)).unwrap();
    let (dev0, prize0, pots0) = (c.dev_pool, c.prize_pool, c.jackpot_pool + c.mini_pool + c.super_pool + c.seed_pool);
    let out = engine::settle(&mut c, &mut p, None, T0 + 1_300).unwrap();
    assert!(out.forfeited && out.spins == 1_000 && out.prizes == 0 && out.jp_hits + out.mini_hits + out.super_hits == 0);
    assert_eq!(p.balance, 0, "the bet of an expired round is not returned to the player");
    // audit No. 3, M2: an expired bet goes entirely to the players' prize bank; the administrator gets nothing from it
    assert_eq!(c.dev_pool, dev0);
    assert_eq!(c.prize_pool - prize0, 1_000 * COST);
    assert_eq!(c.jackpot_pool + c.mini_pool + c.super_pool + c.seed_pool, pots0);
    assert_eq!(ledger(&c, &[&p]), before);
    assert!(engine::settle(&mut c, &mut p, None, T0 + 1_400).is_err(), "cannot be settled twice");
}

#[test]
fn again_spins_are_paid_by_prize_bank_and_bounded() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    let mut rng = Sm(5);
    let (mut extras, mut spins) = (0u64, 0u64);
    for k in 0..3_000u64 {
        engine::deposit(&mut p, 100 * COST, T0).unwrap();
        engine::commit(&mut c, &mut p, T0 + 1, k + 1, 100, false, key(9)).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), T0 + 2).unwrap();
        assert!(out.spins <= 2 * 100 + 16);
        assert_eq!(out.spins, 100 + out.extras);
        extras += out.extras;
        spins += out.spins;
    }
    let share_again = extras as f64 / spins as f64;
    assert!((share_again - 0.116).abs() < 0.004, "{share_again}");
    // with an empty prize bank there are no repeats: there is nothing to pay for them with
    let (mut e, _) = cfg_with(0, 0, 0, 0);
    let mut q = new_player(&mut e, 2, T0);
    engine::deposit(&mut q, COST, T0).unwrap();
    engine::commit(&mut e, &mut q, T0 + 1, 1, 1, false, key(9)).unwrap();
    let out = engine::settle(&mut e, &mut q, Some([3; 32]), T0 + 2).unwrap();
    assert!(out.extras <= 1);
}

// ---------- jackpots ----------
/// Looks for a seed with which a single spin wins the required pot (the pots are chosen so that the chance is noticeable)
fn hunt<F: Fn(&engine::SettleOut) -> bool>(c: &Config, p: &Player, want: F) -> (Config, Player, engine::SettleOut) {
    let mut rng = Sm(42);
    for _ in 0..3_000_000 {
        let (mut c2, mut p2) = (c.clone(), p.clone());
        let out = engine::settle(&mut c2, &mut p2, Some(rng.seed()), T0 + 5).unwrap();
        if want(&out) {
            return (c2, p2, out);
        }
    }
    panic!("the required outcome did not come up in 3 million attempts");
}

#[test]
fn jackpot_pays_95_percent_in_parts_and_restarts_from_admin_amount() {
    // a small starting amount — so that a win is found quickly in the test
    let (mut c, _) = cfg_with(10 * SOL, JP_MIN_POT, 0, 50 * SOL);
    c.mini_pool = 0;
    let mut p = new_player(&mut c, 1, T0);
    engine::deposit(&mut p, COST, T0).unwrap();
    engine::commit(&mut c, &mut p, T0 + 1, 1, 1, false, key(9)).unwrap();
    let total_before = ledger(&c, &[&p]);
    let (c2, mut p2, out) = hunt(&c, &p, |o| o.jp_hits == 1 && o.mini_hits == 0 && o.extras == 0);
    // the win is computed from the whole pot (it is below the pay cap); a fifth of it stays in the pot
    let pot = JP_MIN_POT + share(S_JACK);
    let stay = pot * JP_KEEP_BPS / BPS;
    assert_eq!(out.jp_pool_total, pot - stay, "what left the pot: 80% of it");
    let win = (pot - stay) * JP_WINNER_BPS / BPS;
    assert_eq!(win, pot / 100 * 76, "the winner receives 76% of the pot");
    assert_eq!(out.jp_to_player_now, win * JP_NOW_BPS / BPS, "20% of the winner's share immediately");
    assert_eq!(p2.jp_locked, win - out.jp_to_player_now);
    assert_eq!(c2.super_pool, share(S_SUPER) + (pot - stay) * JP_SUPER_BPS / BPS, "5% of what is paid out goes to the super jackpot");
    assert_eq!(c2.jackpot_pool, JP_MIN_POT, "the new jackpot starts from the administrator's starting amount");
    assert_eq!(c.seed_reserve - c2.seed_reserve, JP_MIN_POT - share(S_SEED) - stay, "top-up from the reserve = starting amount minus what stayed and the accumulated seed");
    assert_eq!(c2.promo_pool, c.promo_pool, "nothing goes to the promotion fund");
    assert_eq!(c2.seed_pool, 0);
    assert_eq!(c2.jackpots_won, 1);
    assert_eq!(ledger(&c2, &[&p2]), total_before);
    // the rest arrives at 8% of the winner's share per week and in full over 10 weeks
    let bal0 = p2.balance;
    engine::touch(&mut p2, T0 + 5 + WEEK_SECS);
    assert_eq!(p2.balance - bal0, win * JP_WEEKLY_BPS / BPS);
    engine::touch(&mut p2, T0 + 5 + 11 * WEEK_SECS);
    assert_eq!(p2.jp_locked, 0);
    assert_eq!(p2.balance - bal0, win - out.jp_to_player_now);
}

#[test]
fn jackpot_without_reserve_restarts_from_collected_seed_only() {
    let (mut c, _) = cfg_with(10 * SOL, JP_MIN_POT, 0, 0);
    c.mini_pool = 0;
    c.seed_pool = 12_345;
    let mut p = new_player(&mut c, 1, T0);
    engine::deposit(&mut p, COST, T0).unwrap();
    engine::commit(&mut c, &mut p, T0 + 1, 1, 1, false, key(9)).unwrap();
    let (c2, _, _) = hunt(&c, &p, |o| o.jp_hits == 1 && o.extras == 0);
    let pot = JP_MIN_POT + share(S_JACK);
    assert_eq!(c2.jackpot_pool, pot * JP_KEEP_BPS / BPS + 12_345 + share(S_SEED), "no reserve: the new jackpot is the fifth that stayed plus the accumulated seed");
    assert!(c2.jackpot_pool < open_pot(JP_MIN_POT), "here it is below a quarter of the starting amount: closed until it has grown");
    assert_eq!(c2.seed_reserve, 0);
}

#[test]
fn mini_and_super_restart_from_admin_amounts() {
    let (mut c, _) = cfg_with(10 * SOL, 0, 0, 50 * SOL);
    c.mini_pool = MINI_MIN_POT;
    let mut p = new_player(&mut c, 1, T0);
    engine::deposit(&mut p, COST, T0).unwrap();
    engine::commit(&mut c, &mut p, T0 + 1, 1, 1, false, key(9)).unwrap();
    let before = ledger(&c, &[&p]);
    let (c2, p2, out) = hunt(&c, &p, |o| o.mini_hits == 1 && o.extras == 0);
    let pot = MINI_MIN_POT + share(S_MINI);
    assert_eq!(out.mini_won, pot - pot * MINI_KEEP_BPS / BPS, "the winner receives two thirds of the mini, a third stays");
    assert_eq!(p2.balance, out.mini_won, "the mini is paid immediately and in full");
    assert_eq!(c.seed_reserve - c2.seed_reserve, MINI_MIN_POT - pot * MINI_KEEP_BPS / BPS, "the reserve adds only what is missing to the starting amount");
    assert_eq!(c2.mini_pool, MINI_MIN_POT, "after a win the mini is topped up to the starting amount from the reserve");
    assert_eq!(ledger(&c2, &[&p2]), before);

    // super jackpot: 90% to the player in parts, 10% stays as the beginning of the next one; the seed reserve is NOT touched (upgrade No. 2)
    let (mut s, _) = cfg_with(10 * SOL, 0, SUPER_MIN_POT, 50 * SOL);
    s.mini_pool = 0;
    let mut w = new_player(&mut s, 2, T0);
    engine::deposit(&mut w, COST, T0).unwrap();
    engine::commit(&mut s, &mut w, T0 + 1, 1, 1, false, key(9)).unwrap();
    let before = ledger(&s, &[&w]);
    let (s2, w2, out) = hunt(&s, &w, |o| o.super_hits == 1 && o.extras == 0);
    let pot = SUPER_MIN_POT + share(S_SUPER);
    assert_eq!(out.super_total, pot * SUPER_WINNER_BPS / BPS);
    assert_eq!(w2.balance, out.super_total * JP_NOW_BPS / BPS);
    assert_eq!(s2.super_pool, pot - out.super_total, "10% of the pot stays as the beginning of the next super jackpot");
    assert_eq!(s2.seed_reserve, s.seed_reserve, "the super jackpot does not take from the seed reserve");
    assert_eq!(s2.supers_won, 1);
    assert_eq!(ledger(&s2, &[&w2]), before);
}

/// The main question about the amount the administrator puts in: can it be "drained" by bets, and what does it cost them.
/// Over a long run a 5 SOL jackpot hits roughly once per 57,000 spins, by the time it is won the pot holds
/// about two starting amounts, and the administrator's reserve spends one starting amount per win.
#[test]
fn admin_jackpot_is_hard_to_win_and_costs_admin_one_start_amount_per_win() {
    let (mut c, _) = cfg_with(2_000 * SOL, 5 * SOL, 20 * SOL, 10_000 * SOL);
    let mut p = new_player(&mut c, 1, T0);
    let mut rng = Sm(2027);
    let reserve0 = c.seed_reserve;
    let (mut exec, mut jp_paid) = (0u64, 0u64);
    for k in 0..60_000u64 {
        engine::deposit(&mut p, 100 * COST, T0).unwrap();
        engine::commit(&mut c, &mut p, T0 + 1, k + 1, 100, false, key(9)).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), T0 + 2).unwrap();
        exec += out.spins;
        jp_paid += out.jp_pool_total;
    }
    let expect = exec as f64 * share(S_JACK) as f64 / (5.0 * SOL as f64);
    let mean_pot = jp_paid as f64 / c.jackpots_won.max(1) as f64 / SOL as f64;
    let spent = (reserve0 - c.seed_reserve) as f64 / exec as f64 / COST as f64;
    println!("spins {exec}: jackpots {} (expected ≈{:.0}), average jackpot {:.1} SOL, mini {}, super {}; the administrator's reserve is spent at {:.2}% of a bet per spin", c.jackpots_won, expect, mean_pot, c.minis_won, c.supers_won, spent * 100.0);
    assert!((c.jackpots_won as f64 - expect).abs() < expect * 0.3, "jackpots {} with {expect} expected", c.jackpots_won);
    assert!(mean_pot > 7.0 && mean_pot < 13.0, "the average jackpot should be about two starting amounts: {mean_pot}");
    // the reserve spent per spin is no more than the sum of the spin's contributions to the three pots (≈1.6% of the bet)
    assert!(spent < (S_JACK + S_MINI + S_SUPER) as f64 / BPS as f64 * 1.25, "the reserve is spent too fast: {spent}");
}

// ---------- conservation of funds ----------
#[test]
fn fuzz_every_operation_keeps_vault_exactly_equal_to_obligations() {
    let (mut c, v0) = cfg_with(30 * SOL, SOL, 3 * SOL, 40 * SOL);
    let mut rng = Sm(2026);
    let mut vault: u128 = v0 as u128;
    let mut now = T0 + 1;
    let mut players: Vec<Player> = (0..40u64).map(|i| new_player(&mut c, i, now)).collect();
    let (mut settled, mut forfeited, mut withdrawn, mut locked) = (0u64, 0u64, 0u128, 0u64);
    for step in 0..60_000u64 {
        let jump = rng.next() % 100;
        now += if jump < 70 { 60 } else if jump < 90 { 3_600 } else if jump < 97 { DAY } else { 9 * DAY };
        let i = (rng.next() % 40) as usize;
        let mut p = std::mem::take(&mut players[i]);
        match rng.next() % 12 {
            0 => {
                let d = (rng.next() % 5_000 + 1) * COST;
                engine::deposit(&mut p, d, now).unwrap();
                vault += d as u128;
            }
            1 => {
                let amount = 1 + rng.next() % p.balance.max(1);
                match engine::withdraw(&mut c, &mut p, amount, now) {
                    Ok((net, fee, burn)) => {
                        assert_eq!((fee, burn), (0, 0));
                        vault -= net as u128;
                        withdrawn += net as u128;
                    }
                    Err(_) => locked += 1,
                }
            }
            2 => {
                let x = c.dev_pool / 2;
                c.dev_pool -= x;
                vault -= x as u128;
            }
            3 => {
                let d = (rng.next() % 3 + 1) * SOL / 10;
                engine::donate(&mut c, d).unwrap();
                vault += d as u128;
            }
            4 => {
                engine::touch(&mut p, now);
            }
            _ => {
                if p.pending {
                    let seed = if rng.next() % 15 == 0 { None } else { Some(rng.seed()) };
                    let out = engine::settle(&mut c, &mut p, seed, now).unwrap();
                    forfeited += out.forfeited as u64;
                    settled += 1;
                } else {
                    let n = [1u16, 1, 2, 5, 10, 50, 100, 1000][(rng.next() % 8) as usize];
                    let mode = (rng.next() % MODES as u64) as u8;
                    let _ = engine::commit_mode(&mut c, &mut p, now, step + 1, n, mode, false, Pubkey::new_unique());
                }
            }
        }
        players[i] = p;
        let refs: Vec<&Player> = players.iter().collect();
        assert_eq!(ledger(&c, &refs), vault, "mismatch at step {step}");
    }
    assert!(settled > 1_000 && forfeited > 10 && withdrawn > 0 && locked > 0);
    assert!(c.jackpots_won > 0 && c.minis_won > 0, "the test must involve pot wins: {} {} {}", c.jackpots_won, c.minis_won, c.supers_won);
    assert_eq!(c.total_burned + c.promo_pool + c.act_pool, 0);
}

#[test]
fn settled_batch_of_a_thousand_conserves_money() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    engine::deposit(&mut p, 1_000 * COST, T0).unwrap();
    let before = ledger(&c, &[&p]);
    engine::commit(&mut c, &mut p, T0 + 1, 1, 1_000, false, key(9)).unwrap();
    let out = engine::settle(&mut c, &mut p, Some([7; 32]), T0 + 2).unwrap();
    assert!(out.spins >= 1_000);
    assert_eq!(ledger(&c, &[&p]), before);
}

/// A long run on every mode with a full bank: where the paid bets go.
/// 6.99% to the administrator (618 bps of every executed spin), the rest to players (paid out + held in the pots, which go to players).
#[test]
fn long_run_split_is_ninety_to_players_ten_to_admin_in_every_game() {
    for mode in 0..MODES as u8 {
        let (mut c, v0) = cfg_with(100_000 * SOL, 5 * SOL, 20 * SOL, 1_000 * SOL);
        let mut p = new_player(&mut c, 1, T0);
        let mut rng = Sm(900 + mode as u64);
        let (mut paid, mut taken) = (0u128, 0u128);
        let batch = 100u16;
        for k in 0..20_000u64 {
            p.balance += batch as u64 * COST;
            paid += batch as u128 * COST as u128;
            engine::commit_mode(&mut c, &mut p, T0 + 1, k + 1, batch, mode, false, key(9)).unwrap();
            engine::settle(&mut c, &mut p, Some(rng.seed()), T0 + 2).unwrap();
            taken += p.balance as u128;
            p.balance = 0;
        }
        let admin = c.dev_pool as f64 / paid as f64;
        let rtp = taken as f64 / paid as f64;
        let pots = (c.jackpot_pool + c.mini_pool + c.super_pool + c.seed_pool + c.seed_reserve) as f64;
        let pots0 = (v0 - 100_000 * SOL) as f64;
        println!("mode {mode}: paid to players {:.2}%, to the administrator {:.2}%, growth of the jackpot pots {:+.2}%", rtp * 100.0, admin * 100.0, (pots - pots0) / paid as f64 * 100.0);
        assert!((admin - 0.0699).abs() < 0.0012, "mode {mode}: administrator {admin}");
        // rare big prizes cause variance; the average is 87.7% in regular prizes plus the jackpots
        assert!(rtp > 0.80 && rtp < 0.97, "mode {mode}: return {rtp}");
    }
}

#[test]
fn empty_bank_never_owes_more_than_it_holds() {
    let (mut c, _) = cfg_with(0, 0, 0, 0);
    let mut p = new_player(&mut c, 1, T0);
    let mut rng = Sm(2026);
    let mut now = T0 + 1;
    let (mut paid, mut taken) = (0u128, 0u128);
    for k in 0..300_000u64 {
        now += 300;
        p.balance += 10 * COST;
        paid += 10 * COST as u128;
        engine::commit(&mut c, &mut p, now, k + 1, 10, false, key(9)).unwrap();
        engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
        taken += p.balance as u128;
        p.balance = 0;
        let held = c.prize_pool as u128 + c.jackpot_pool as u128 + c.mini_pool as u128 + c.super_pool as u128 + c.seed_pool as u128
            + c.seed_reserve as u128 + c.dev_pool as u128 + p.jp_locked as u128;
        assert_eq!(c.promo_pool, 0);
        assert_eq!(paid, taken + held, "step {k}: funds neither appear nor disappear");
    }
    engine::touch(&mut p, now + 400 * DAY);
    taken += p.balance as u128;
    let rtp = taken as f64 / paid as f64;
    let admin = c.dev_pool as f64 / paid as f64;
    println!("empty bank: paid to players {:.2}%, to the administrator {:.2}%, left in the pots {:.2}%", rtp * 100.0, admin * 100.0, (1.0 - rtp - admin) * 100.0);
    assert!((admin - 0.0699).abs() < 0.0012, "administrator {admin}");
    // long-run ceiling: (7,750 table + 443 pots) ÷ 8,840 = 92.7% of paid bets (it was 89.7% with 10% to the administrator)
    assert!(rtp > 0.72 && rtp < 0.93, "return to players {rtp}");
    assert_eq!(c.promo_pool + c.act_pool + c.total_burned, 0);
}

// ---------- week, streak ----------
#[test]
fn epoch_roll_never_moves_money() {
    // previously the prize bank surplus went to the jackpot once a week: this made it possible to pump the bank into the pot (audit K2, S1)
    let mut c = funded_cfg();
    c.prize_buffer = 1;
    c.epoch_spins = 12_345;
    let before = (c.prize_pool, c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_reserve);
    assert!(engine::roll_epoch_if_due(&mut c, T0 + EPOCH_SECS - 1).is_none());
    let e = engine::roll_epoch_if_due(&mut c, T0 + EPOCH_SECS).unwrap();
    assert_eq!(e.moved_to_jackpot, 0);
    assert_eq!((c.prize_pool, c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_reserve), before);
    assert_eq!((c.den, c.epoch_spins), (12_345, 0));
}

// ---------- protection against the administrator (audit of 11.11.2026: K1, K2) ----------
#[test]
fn start_amounts_change_only_after_a_day_and_within_bounds() {
    let mut c = funded_cfg(); // jackpot 5 SOL, super 20 SOL
    let now = T0 + 100;
    assert!(engine::set_starts(&mut c, JP_MIN_POT - 1, 20 * SOL, now).is_err(), "below the smallest amount is not allowed");
    assert!(engine::set_starts(&mut c, 5 * SOL, SUPER_MIN_POT - 1, now).is_err());
    assert!(engine::set_starts(&mut c, START_MAX + 1, 20 * SOL, now).is_err(), "an unreachable amount cannot be set");
    assert!(engine::set_starts(&mut c, 2 * SOL, 20 * SOL, now).is_err(), "the amount cannot be lowered below half of the pot (it holds 5 SOL)");
    assert!(engine::set_starts(&mut c, 5 * SOL, 9 * SOL, now).is_err(), "the same for the super jackpot (it holds 20 SOL)");
    engine::set_starts(&mut c, 3 * SOL, 12 * SOL, now).unwrap();
    assert_eq!((c.init_jackpot, c.init_super), (5 * SOL, 20 * SOL), "nothing changes right away");
    assert_eq!(c.starts_apply_ts, now + STARTS_DELAY_SECS);
    // a bet before the deadline locks in the previous amounts
    let mut p = new_player(&mut c, 1, T0);
    engine::deposit(&mut p, 10 * COST, T0).unwrap();
    engine::commit(&mut c, &mut p, now + STARTS_DELAY_SECS - 1, 1, 1, false, key(9)).unwrap();
    assert_eq!((p.p_start_jackpot, p.p_start_super), (5 * SOL, 20 * SOL));
    // settlement after the deadline: the config is already new, but this round is settled at the locked-in amounts
    engine::settle(&mut c, &mut p, Some([1; 32]), now + STARTS_DELAY_SECS + 5).unwrap();
    assert_eq!((c.init_jackpot, c.init_super, c.starts_apply_ts), (3 * SOL, 12 * SOL, 0));
    engine::commit(&mut c, &mut p, now + STARTS_DELAY_SECS + 10, 2, 1, false, key(9)).unwrap();
    assert_eq!((p.p_start_jackpot, p.p_start_super), (3 * SOL, 12 * SOL), "new bets — at the new amounts");
}

#[test]
fn admin_cannot_change_odds_of_a_bet_already_placed() {
    // Audit scenario K1: see the randomness of one's own bet and lower the starting amounts before settlement.
    // Now the amounts can be changed only with a delay, and settlement takes the amounts locked in at bet time.
    let (mut c, _) = cfg_with(100 * SOL, 5 * SOL, 20 * SOL, 100 * SOL);
    let mut p = new_player(&mut c, 1, T0);
    engine::deposit(&mut p, 1_000 * COST, T0).unwrap();
    engine::commit(&mut c, &mut p, T0 + 1, 1, 1_000, false, key(9)).unwrap();
    let mut rng = Sm(7);
    let (mut tot_h, mut tot_e, mut with_pot) = ((0u32, 0u64), (0u32, 0u64), 0u32);
    for _ in 0..200 {
        let seed = rng.seed();
        let (mut a, mut pa) = (c.clone(), p.clone());
        let honest = engine::settle(&mut a, &mut pa, Some(seed), T0 + 5).unwrap();
        // the "evil" administrator: edits the config directly before settlement, as an instant command would
        let (mut b, mut pb) = (c.clone(), p.clone());
        b.init_jackpot = JP_MIN_POT;
        b.init_super = SUPER_MIN_POT;
        let evil = engine::settle(&mut b, &mut pb, Some(seed), T0 + 5).unwrap();
        // the chances come from the locked-in amounts: lowering the amounts in the config does not add a single pot win
        // (only the amount to which a pot is topped up after a win changes — and it becomes smaller)
        // Exact statement: the config amounts are read only AFTER a pot win (the top-up), so up to and including the first pot
        // win of the round the two settlements are identical; a round without a pot win is identical as a whole.
        let first = |o: &engine::SettleOut| (o.jp_hits + o.mini_hits + o.super_hits == 0, o.prizes, o.spins);
        if honest.jp_hits + honest.mini_hits + honest.super_hits == 0 {
            assert_eq!(first(&honest), first(&evil), "a round without a pot win does not depend on the config amounts");
            assert_eq!((a.jackpot_pool, a.mini_pool, a.super_pool, pa.balance), (b.jackpot_pool, b.mini_pool, b.super_pool, pb.balance));
        } else {
            assert!(evil.jp_hits + evil.mini_hits + evil.super_hits > 0, "the first pot win is the same in both");
            with_pot += 1;
        }
        // after a win the "evil" pots are topped up to SMALLER amounts and are closed while the locked-in amounts are in force:
        // over all the rounds the lowered amounts bring fewer pot wins and less money, not more
        tot_h = (tot_h.0 + honest.jp_hits + honest.mini_hits, tot_h.1 + honest.jp_pool_total + honest.mini_won);
        tot_e = (tot_e.0 + evil.jp_hits + evil.mini_hits, tot_e.1 + evil.jp_pool_total + evil.mini_won);
    }
    assert!(with_pot > 20 && tot_e.0 <= tot_h.0 && tot_e.1 <= tot_h.1, "lowering the starting amounts before settlement must not add pot wins: {tot_h:?} / {tot_e:?} in {with_pot} rounds");
}

#[test]
fn late_settlement_pays_prizes_but_not_pots() {
    // The oracle answered but nobody completed the round within 20 minutes: regular prizes are kept, the pots are not played (audit S4)
    let (mut c, _) = cfg_with(100 * SOL, JP_MIN_POT, 0, 50 * SOL);
    c.mini_pool = 0;
    let mut p = new_player(&mut c, 1, T0);
    engine::deposit(&mut p, 100 * COST, T0).unwrap();
    engine::commit(&mut c, &mut p, T0 + 1, 1, 100, false, key(9)).unwrap();
    let before = ledger(&c, &[&p]);
    let mut rng = Sm(11);
    let mut seen = 0;
    for _ in 0..400_000 {
        let seed = rng.seed();
        let (mut a, mut pa) = (c.clone(), p.clone());
        let on_time = engine::settle(&mut a, &mut pa, Some(seed), T0 + 5).unwrap();
        if on_time.jp_hits == 0 {
            continue;
        }
        seen += 1;
        let (mut b, mut pb) = (c.clone(), p.clone());
        let late = engine::settle_late(&mut b, &mut pb, seed, T0 + 2_000).unwrap();
        assert_eq!((late.jp_hits, late.mini_hits, late.super_hits), (0, 0, 0));
        assert!(!late.forfeited && late.spins >= 100);
        assert_eq!(pb.jp_locked, 0);
        assert_eq!(ledger(&b, &[&pb]), before);
        assert!(b.jackpot_pool > c.jackpot_pool, "the pot stayed in place and grew by the round's contributions");
        if seen >= 3 {
            break;
        }
    }
    assert!(seen >= 1, "no round with a jackpot was encountered in 400 thousand attempts");
    // without an oracle answer the round still expires
    let out = engine::settle(&mut c, &mut p, None, T0 + 2_000).unwrap();
    assert!(out.forfeited && out.prizes == 0);
}

#[test]
fn one_prize_cannot_take_more_than_a_quarter_of_the_bank() {
    let (mut c, _) = cfg_with(2 * SOL, 0, 0, 0);
    let mut p = new_player(&mut c, 1, T0);
    let mut rng = Sm(3);
    for k in 0..40_000u64 {
        let bank = c.prize_pool;
        engine::deposit(&mut p, COST, T0).unwrap();
        engine::commit_mode(&mut c, &mut p, T0 + 1, k + 1, 1, 12, false, key(9)).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), T0 + 2).unwrap();
        assert!(out.best_prize <= (bank + out.spins * share(S_PRIZE)) / PRIZE_BANK_DIV, "prize {} with bank {bank}", out.best_prize);
        p.balance = 0;
    }
    assert!(c.prize_pool > 0);
}

/// Equality of games at a "real-life" launch: bank 50 SOL, jackpot 5, super 20, reserve 30 (audit S2).
/// Previously the games with rare ×10,000 prizes returned 7% less than the others; now the top prize is ×1000.
#[test]
fn games_return_the_same_with_a_realistic_bank() {
    let mut rtps = vec![];
    for mode in 0..MODES as u8 {
        let (mut c, _) = cfg_with(50 * SOL, 5 * SOL, 20 * SOL, 30 * SOL);
        let mut p = new_player(&mut c, 1, T0);
        let mut rng = Sm(4_000 + mode as u64);
        let (mut paid, mut prizes) = (0u128, 0u128);
        for k in 0..30_000u64 {
            p.balance += 100 * COST;
            paid += 100 * COST as u128;
            engine::commit_mode(&mut c, &mut p, T0 + 1 + k as i64 * 20, k + 1, 100, mode, false, key(9)).unwrap();
            prizes += engine::settle(&mut c, &mut p, Some(rng.seed()), T0 + 2 + k as i64 * 20).unwrap().prizes as u128;
            p.balance = 0;
        }
        let r = prizes as f64 / paid as f64;
        println!("bank 50 SOL, mode {mode}: {:.2}% of paid bets paid out in regular prizes, {:.1} SOL left in the bank", r * 100.0, c.prize_pool as f64 / SOL as f64);
        rtps.push(r);
    }
    let (lo, hi) = (rtps.iter().cloned().fold(1.0, f64::min), rtps.iter().cloned().fold(0.0, f64::max));
    assert!(hi - lo < 0.03, "spread of return between games {:.2}%", (hi - lo) * 100.0);
    assert!(lo > 0.84 && hi < 0.90, "return in regular prizes: from {lo} to {hi}");
}

#[test]
fn missing_a_day_resets_streak_but_keeps_money() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 5 * SOL;
    p.run_spins = 500;
    engine::touch(&mut p, T0 + DAY);
    assert_eq!(p.streak, 2);
    let t = engine::touch(&mut p, T0 + 4 * DAY);
    assert!(t.burned && p.streak == 1 && p.run_spins == 0);
    assert_eq!(p.balance, 5 * SOL);
}

#[test]
fn registration_is_sequential_and_needs_open_bank() {
    let mut c = funded_cfg();
    assert_eq!((engine::assign_number(&mut c).unwrap(), engine::assign_number(&mut c).unwrap()), (1, 2));
    let mut u = Config::default();
    assert!(engine::assign_number(&mut u).is_err());
}

#[test]
fn initialize_is_bound_to_this_programs_own_code_account() {
    use std::str::FromStr;
    let meta = Pubkey::from_str("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s").unwrap();
    let meta_data = Pubkey::from_str("PwDiXFxQsGra4sFFTT8r1QWRMd4vfumiWC1jfWNfdYT").unwrap();
    assert_eq!(fairdrum::program_data_address(&meta), meta_data, "the address formula did not match the network");
    assert!(fairdrum::is_own_program_data(&meta, &meta_data));
    let ours = Pubkey::new_unique();
    assert!(!fairdrum::is_own_program_data(&ours, &meta_data));
    assert!(!fairdrum::is_own_program_data(&ours, &ours));
    assert!(fairdrum::is_own_program_data(&ours, &fairdrum::program_data_address(&ours)));
}

// ---------- audit No. 3 ----------

/// S1: the prize cap is fixed at the moment of the bet — topping up the bank after the bet does not yield a bigger prize.
#[test]
fn prize_cap_is_fixed_at_commit() {
    for seed_i in 0..4_000u64 {
        let mk = || {
            let mut c = funded_cfg();
            c.prize_pool = 2 * SOL; // small bank: prize cap 0.5 SOL
            c.seed_reserve = 500 * SOL;
            let mut p = new_player(&mut c, 1, T0);
            engine::deposit(&mut p, 100 * COST, T0).unwrap();
            engine::commit_mode(&mut c, &mut p, T0, 1, 100, 2, false, key(9)).unwrap();
            (c, p)
        };
        let mut sd = [0u8; 32];
        sd[..8].copy_from_slice(&seed_i.to_le_bytes());
        let (mut a, mut pa) = mk();
        let fair = engine::settle(&mut a, &mut pa, Some(sd), T0 + 10).unwrap();
        let (mut b, mut pb) = mk();
        engine::rescale(&mut b, 400 * SOL, 0, 0).unwrap(); // the administrator fills the bank after the bet
        let pumped = engine::settle(&mut b, &mut pb, Some(sd), T0 + 10).unwrap();
        assert!(pumped.best_prize <= fair.best_prize.max(SOL / 2), "the prize grew after the bank was filled: {} > {}", pumped.best_prize, fair.best_prize);
        assert!(pumped.best_prize <= SOL / 2, "the prize is above the cap fixed at bet time");
    }
}

/// M1: a late settlement is never more profitable than a timely one — the same sectors, but the pots do not pay.
#[test]
fn late_settlement_is_never_better_than_on_time() {
    let mut better = 0u32;
    let mut pot_hits = 0u32;
    for seed_i in 0..20_000u64 {
        let mk = || {
            let mut c = funded_cfg();
            c.init_jackpot = JP_MIN_POT; // smallest starting amounts: the pots hit most often
            c.init_super = SUPER_MIN_POT;
            c.jackpot_pool = JP_MIN_POT;
            c.super_pool = SUPER_MIN_POT;
            let mut p = new_player(&mut c, 1, T0);
            engine::deposit(&mut p, 100 * COST, T0).unwrap();
            engine::commit_mode(&mut c, &mut p, T0, 1, 100, 0, false, key(9)).unwrap();
            (c, p)
        };
        let mut sd = [0u8; 32];
        sd[..8].copy_from_slice(&seed_i.to_le_bytes());
        let (mut a, mut pa) = mk();
        let on = engine::settle(&mut a, &mut pa, Some(sd), T0 + 10).unwrap();
        let (mut b, mut pb) = mk();
        let late = engine::settle_late(&mut b, &mut pb, sd, T0 + 2_000).unwrap();
        assert_eq!(late.jp_hits + late.mini_hits + late.super_hits, 0);
        pot_hits += (on.jp_hits + on.mini_hits + on.super_hits) as u32;
        if pb.balance + pb.jp_locked > pa.balance + pa.jp_locked {
            better += 1;
        }
    }
    assert!(pot_hits > 0, "the sample must contain rounds with pots");
    assert_eq!(better, 0, "a late settlement turned out more profitable than a timely one");
}

/// Public statistics: turnover, credits to players and the administrator, the list of recent pot wins.
/// The counters match what actually happened to the balances and do not affect settlements.
#[test]
fn public_stats_match_real_money_flow() {
    let mut c = funded_cfg();
    c.init_jackpot = JP_MIN_POT; // small pots: won often, the list fills up
    c.init_super = SUPER_MIN_POT;
    c.jackpot_pool = JP_MIN_POT;
    c.super_pool = SUPER_MIN_POT;
    let mut p = new_player(&mut c, 1, T0);
    engine::deposit(&mut p, 2_000_000 * COST, T0).unwrap();
    let (dev0, mut staked, mut credited) = (c.dev_pool, 0u64, 0u64);
    let mut pots = 0u64;
    for i in 0..6_000u64 {
        let (bal0, lock0) = (p.balance, p.jp_locked);
        engine::commit_mode(&mut c, &mut p, T0 + i as i64, 1, 100, (i % MODES as u64) as u8, false, key(9)).unwrap();
        staked += 100 * COST;
        let mut sd = [0u8; 32];
        sd[..8].copy_from_slice(&(i * 7 + 3).to_le_bytes());
        let out = engine::settle(&mut c, &mut p, Some(sd), T0 + i as i64).unwrap();
        credited += p.balance + p.jp_locked + 100 * COST - bal0 - lock0;
        pots += (out.jp_hits + out.mini_hits + out.super_hits) as u64;
    }
    assert_eq!(c.staked_total, staked);
    assert_eq!(c.dev_total, c.dev_pool - dev0);
    assert_eq!(c.paid_total, credited, "credited to players per the counter and per the balances");
    assert!(pots >= 8, "the sample must contain at least eight pot wins, there were {pots}");
    assert!(c.wins.iter().all(|w| w.kind >= 1 && w.kind <= 3 && w.amount > 0 && w.player == p.owner));
    assert_eq!(c.wins_head as u64, pots % 8);
    assert_eq!(c.mode_rounds.iter().sum::<u64>(), 6_000);
}

// ---------------- round salt ----------------

fn slot_hashes_data(slots: &[u64]) -> Vec<u8> {
    let mut d = (slots.len() as u64).to_le_bytes().to_vec();
    for s in slots {
        d.extend_from_slice(&s.to_le_bytes());
        d.extend_from_slice(&[(*s % 251) as u8; 32]);
    }
    d
}

#[test]
fn slot_hash_lookup_handles_skipped_slots_and_missing_ones() {
    // descending, with gaps (slots 995, 990..992 were not produced)
    let slots: Vec<u64> = (480..=1000u64).rev().filter(|s| *s != 995 && !(990..=992).contains(s)).collect();
    let d = slot_hashes_data(&slots);
    for s in &slots {
        assert_eq!(fairdrum::math::slot_hash_of(&d, *s), Some([(*s % 251) as u8; 32]), "slot {s}");
    }
    for s in [995u64, 990, 991, 992, 479, 1001, 0, u64::MAX] {
        assert_eq!(fairdrum::math::slot_hash_of(&d, s), None, "slot {s} is absent");
    }
    assert_eq!(fairdrum::math::slot_hash_of(&[], 5), None);
    assert_eq!(fairdrum::math::slot_hash_of(&slot_hashes_data(&[]), 5), None);
    // the declared length exceeds the data — we do not go out of bounds
    let mut bad = slot_hashes_data(&[10, 9, 8]);
    bad[0] = 200;
    assert_eq!(fairdrum::math::slot_hash_of(&bad, 9), Some([9u8; 32]));
}

#[test]
fn salted_value_depends_on_both_inputs() {
    let v = [7u8; 32];
    let a = fairdrum::math::salted(v, &[1u8; 32]);
    let b = fairdrum::math::salted(v, &[2u8; 32]);
    let c = fairdrum::math::salted([8u8; 32], &[1u8; 32]);
    assert_ne!(a, b, "a different salt — a different number");
    assert_ne!(a, c, "a different oracle number — a different round number");
    assert_ne!(a, v);
    assert_eq!(a, fairdrum::math::salted(v, &[1u8; 32]), "deterministic: anyone can verify");
}


// ---------------- house secret tree ----------------

fn tree(depth: u8, master: u8) -> (Vec<[u8; 32]>, Vec<Vec<[u8; 32]>>) {
    let n = 1usize << depth;
    let secrets: Vec<[u8; 32]> = (0..n).map(|i| solana_sha256_hasher::hashv(&[b"fairdrum-house-leaf", &[master; 32], &(i as u64).to_le_bytes()]).to_bytes()).collect();
    let mut levels = vec![secrets.iter().enumerate().map(|(i, s)| fairdrum::math::house_leaf(i as u64, s)).collect::<Vec<_>>()];
    while levels.last().unwrap().len() > 1 {
        let l = levels.last().unwrap();
        levels.push(l.chunks(2).map(|c| solana_sha256_hasher::hashv(&[&[1u8], &c[0], &c[1]]).to_bytes()).collect());
    }
    (secrets, levels)
}
fn proof(levels: &[Vec<[u8; 32]>], mut i: usize) -> Vec<[u8; 32]> {
    let mut out = vec![];
    for l in &levels[..levels.len() - 1] {
        out.push(l[i ^ 1]);
        i >>= 1;
    }
    out
}

#[test]
fn house_tree_every_leaf_proves_and_nothing_else_does() {
    let (secrets, levels) = tree(6, 7);
    let root = levels.last().unwrap()[0];
    for i in 0..secrets.len() {
        let pr = proof(&levels, i);
        assert_eq!(fairdrum::math::house_root_of(i as u64, &secrets[i], &pr), root, "leaf {i}");
        // someone else's secret under this number does not fit
        let j = (i + 1) % secrets.len();
        assert_ne!(fairdrum::math::house_root_of(i as u64, &secrets[j], &pr), root);
        // one's own secret under someone else's number does not fit (the number is part of the leaf hash)
        assert_ne!(fairdrum::math::house_root_of(j as u64, &secrets[i], &pr), root);
        assert_ne!(fairdrum::math::house_root_of(j as u64, &secrets[i], &proof(&levels, j)), root);
        // a corrupted path does not fit
        let mut bad = pr.clone();
        bad[i % 6][0] ^= 1;
        assert_ne!(fairdrum::math::house_root_of(i as u64, &secrets[i], &bad), root);
        // a shortened and a lengthened path do not fit
        assert_ne!(fairdrum::math::house_root_of(i as u64, &secrets[i], &pr[..5]), root);
        let mut long = pr.clone();
        long.push([0u8; 32]);
        assert_ne!(fairdrum::math::house_root_of(i as u64, &secrets[i], &long), root);
    }
    // a tree node cannot be passed off as a leaf: a leaf and a node have different prefixes
    let node = levels[1][0];
    assert_ne!(fairdrum::math::house_root_of(0, &node, &proof(&levels, 0)[1..]), root);
    // a different master secret — a different tree
    assert_ne!(tree(6, 8).1.last().unwrap()[0], root);
}

#[test]
fn round_value_needs_both_secret_and_block_hash() {
    let (secrets, _) = tree(3, 1);
    let v = fairdrum::math::salted(secrets[0], &[5u8; 32]);
    assert_ne!(v, fairdrum::math::salted(secrets[0], &[6u8; 32]), "a different block — a different number");
    assert_ne!(v, fairdrum::math::salted(secrets[1], &[5u8; 32]), "a different secret — a different number");
}

#[test]
fn refund_returns_exactly_the_stake_and_touches_nothing_else() {
    let mut cfg = funded_cfg();
    let mut p = new_player(&mut cfg, 1, T0);
    fairdrum::engine::deposit(&mut p, 500 * COST, T0).unwrap();
    let (bal0, prize0, dev0, jack0, staked0) = (p.balance, cfg.prize_pool, cfg.dev_pool, cfg.jackpot_pool, cfg.staked_total);
    fairdrum::engine::commit_mode(&mut cfg, &mut p, T0 + 10, 100, 100, 3, false, Pubkey::new_unique()).unwrap();
    assert_eq!(p.balance, bal0 - 100 * COST);
    let out = fairdrum::engine::refund(&mut cfg, &mut p, T0 + 2000).unwrap();
    assert!(out.refunded && !out.forfeited);
    assert_eq!(p.balance, bal0, "the bet was returned in full");
    assert!(!p.pending && p.p_stake == 0);
    assert_eq!((cfg.prize_pool, cfg.dev_pool, cfg.jackpot_pool, cfg.staked_total), (prize0, dev0, jack0, staked0), "the pools and the statistics did not change");
    assert_eq!(cfg.refunds, 1, "the refund is counted in the public counter");
    assert!(fairdrum::engine::refund(&mut cfg, &mut p, T0 + 2001).is_err(), "cannot be refunded a second time");
}

#[test]
fn deposits_are_never_frozen_and_the_lock_fields_stay_consistent() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    let h = 3_600;
    engine::deposit(&mut p, 10 * SOL, T0).unwrap();
    assert_eq!((p.dep_locked, p.dep_unlock_ts, p.last_deposit_ts), (10 * SOL, T0, T0), "the fields are still written (same layout); the term is zero");
    p.balance += 4 * SOL; // win
    assert_eq!(engine::withdrawable(&p, T0), 14 * SOL);
    // lost part of the deposit: what is left is free
    p.balance = 3 * SOL;
    assert_eq!(engine::withdrawable(&p, T0 + h), 3 * SOL);
    // many small deposits (someone else's "pennies" through the deposit address) freeze nothing and overflow nothing
    for k in 0..100 {
        engine::deposit(&mut p, SOL / 1000, T0 + 11 * h + k).unwrap();
        assert_eq!(engine::withdrawable(&p, T0 + 11 * h + k), p.balance);
        assert_eq!(p.dep_unlock_ts, T0 + 11 * h + k);
    }
    // the largest amounts: no overflow in the term arithmetic
    let mut q = new_player(&mut c, 2, T0);
    engine::deposit(&mut q, u64::MAX / 2, T0).unwrap();
    engine::deposit(&mut q, u64::MAX / 2, T0).unwrap();
    assert_eq!(engine::withdrawable(&q, T0), q.balance);
    assert!(engine::deposit(&mut q, 2, T0).is_err(), "a balance overflow is still an error");
}

// ---------- upgrade No. 2: secret root, round sizes ----------
#[test]
fn house_root_rotation_keeps_history_and_rejects_reuse() {
    let mut c = funded_cfg();
    let unlock0 = c.super_unlock_ts;
    assert!(engine::set_root(&mut c, [0; 32], 10).is_err(), "a zero root is not allowed");
    assert!(engine::set_root(&mut c, [1; 32], 0).is_err(), "depth from 1");
    assert!(engine::set_root(&mut c, [1; 32], HOUSE_MAX_DEPTH + 1).is_err(), "depth no greater than the maximum");
    engine::set_root(&mut c, [1; 32], 10).unwrap();
    assert_eq!((c.house_root, c.house_depth, c.house_start, c.house_prev_head), ([1; 32], 10, 0, 0));
    assert!(engine::set_root(&mut c, [1; 32], 12).is_err(), "the root in effect cannot be set again");
    c.round_counter = 77;
    engine::set_root(&mut c, [2; 32], 12).unwrap();
    assert_eq!((c.house_root, c.house_depth, c.house_start), ([2; 32], 12, 77), "secret numbering starts from the current bet");
    assert_eq!((c.house_prev[0], c.house_prev_head), ([1; 32], 1), "the previous root is written to the ring");
    assert!(engine::set_root(&mut c, [1; 32], 10).is_err(), "a previous root cannot be brought back");
    assert_eq!(c.super_unlock_ts, unlock0, "the time field is not touched");
}

/// Change D: `engine::set_root` — the same write that the set_house_root command of the live program performed.
/// The reference below is the body of the live command word for word (programs/fairdrum/src/lib.rs before the upgrade).
#[test]
fn set_root_equals_live_set_house_root() {
    fn live(cfg: &mut Config, root: [u8; 32], depth: u8) -> bool {
        if !(depth >= 1 && depth <= HOUSE_MAX_DEPTH && root != [0u8; 32]) {
            return false;
        }
        if !(root != cfg.house_root && !cfg.house_prev.contains(&root)) {
            return false;
        }
        if cfg.house_depth > 0 {
            let i = (cfg.house_prev_head as usize) % cfg.house_prev.len();
            cfg.house_prev[i] = cfg.house_root;
            cfg.house_prev_head = ((i + 1) % cfg.house_prev.len()) as u8;
        }
        cfg.house_root = root;
        cfg.house_depth = depth;
        cfg.house_start = cfg.round_counter;
        true
    }
    let bytes = |c: &Config| {
        let mut v = Vec::new();
        anchor_lang::AccountSerialize::try_serialize(c, &mut v).unwrap();
        v
    };
    let (mut a, mut b) = (funded_cfg(), funded_cfg());
    let mut x = 0x00D0_5EEDu64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let (mut ok, mut rejected) = (0, 0);
    for step in 0..20_000u64 {
        // roots from a small set (so that there are repeats and rejections), a zero root, depths 0…22, "bets" between changes
        let r = [(next() % 14) as u8; 32];
        let depth = (next() % 23) as u8;
        if next() % 3 == 0 {
            a.round_counter += next() % 1000;
            b.round_counter = a.round_counter;
        }
        // on-chain, state after a failed command is rolled back: we compare only after successful ones, and a rejection — on a copy
        let (mut a2, mut b2) = (a.clone(), b.clone());
        let ra = engine::set_root(&mut a2, r, depth).is_ok();
        let rb = live(&mut b2, r, depth);
        assert_eq!(ra, rb, "step {step}: the decision diverged");
        if ra {
            a = a2;
            b = b2;
            ok += 1;
        } else {
            rejected += 1;
        }
        assert_eq!(bytes(&a), bytes(&b), "step {step}: the config account diverged");
    }
    assert!(ok > 1_000 && rejected > 1_000, "accepted {ok}, rejected {rejected}");
}

#[test]
fn round_size_bounds_are_consistent() {
    // the smallest round is no larger than any game's limit, the session key limit is between them
    for m in 0..MODES as u8 {
        assert!(MIN_BATCH <= batch_limit_mode(m, 0));
    }
    assert!(MIN_BATCH <= SESSION_MAX_BATCH && SESSION_MAX_BATCH <= BATCH_LIMIT);
    assert_eq!((MIN_BATCH as u64) * COST, SOL / 10, "smallest round — 0.1 SOL");
    assert_eq!((SESSION_MAX_BATCH as u64) * COST, SOL, "session key — no more than 1 SOL per round");
}
