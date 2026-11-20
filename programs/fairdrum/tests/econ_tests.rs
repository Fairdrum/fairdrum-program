//! Economics of upgrade No. 2 (owner's decision of 18.11.2026): the administrator's share 10% → 7% of paid bets, the freed part goes
//! to the super jackpot; a pot win is computed from a bounded amount, the super jackpot is not topped up from the reserve.
//! Everything here is computed from the program's own constants and functions; the amounts are exact integers (lamports),
//! floating point is used only to print percentages and in the statistical checks.
//!
//!   cargo test --release -p fairdrum --test econ_tests -- --nocapture
//!
//! The long simulation settles 12 million paid bets (120,000 SOL); FD_ECON_STAKES=<n> changes the number.

use anchor_lang::prelude::Pubkey;
use fairdrum::consts::*;
use fairdrum::engine;
use fairdrum::math::*;
use fairdrum::state::*;
mod v1ref;

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
    fn unit(&mut self) -> f64 {
        ((self.next() >> 11) as f64 + 0.5) / (1u64 << 53) as f64
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
    b[31] = 3;
    Pubkey::new_from_array(b)
}
fn bank(prize: u64, jackpot: u64, superj: u64, reserve: u64) -> (Config, u64) {
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
fn pools(c: &Config) -> u128 {
    [c.act_pool, c.prize_pool, c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_pool, c.seed_reserve, c.dev_pool, c.promo_pool].iter().map(|x| *x as u128).sum()
}
fn owed(p: &Player) -> u128 {
    p.balance as u128 + p.jp_locked as u128 + if p.pending { p.p_stake as u128 } else { 0 }
}
fn pct(num: u128, den: u128) -> f64 {
    num as f64 / den as f64 * 100.0
}

// ---------------------------------------------------------------------------------------------------------------------------
// 1. What "7%" is, exactly
// ---------------------------------------------------------------------------------------------------------------------------

/// Shares are taken from every EXECUTED spin; a paid bet gives 1/(1 − 0.116) executed spins on average (each spin is a free
/// repeat with probability W_AGAIN). So a share of s bps per executed spin is s/0.884 bps of the PAID bets:
/// v1: 884/0.884 = 1000 bps exactly (10%); 7% would need 618.8; a share is a whole number of bps → 618 (6.991%), 619 is 7.0023%.
#[test]
fn admin_share_is_seven_percent_of_paid_volume() {
    assert_eq!((S_PRIZE, S_JACK, S_MINI, S_SUPER, S_SEED, S_DEV, S_BURN, S_ACT), (8_939, 88, 44, 293, 18, 618, 0, 0));
    assert_eq!(S_PRIZE + S_JACK + S_MINI + S_SUPER + S_SEED + S_DEV, BPS);
    // per paid bet, as an exact fraction: s × BPS / (BPS − W_AGAIN) bps
    let per_paid = |s: u64| (s as u128 * BPS as u128, (BPS - W_AGAIN) as u128); // numerator, denominator (bps)
    let (n1, d) = per_paid(V1_S_DEV);
    assert_eq!(n1, 1_000 * d, "v1: exactly 10% of paid bets");
    let (n, d) = per_paid(S_DEV);
    assert!(n <= 700 * d && per_paid(S_DEV + 1).0 > 700 * d, "618 is the largest whole share that does not exceed 7% of paid bets");
    assert_eq!((n * 1_000 / d, per_paid(S_DEV + 1).0 * 1_000 / d), (699_095, 700_226), "618 → 699.095 bps, 619 → 700.226 bps of paid bets");
    // what the administrator gives up goes to the super jackpot, lamport for lamport
    assert_eq!(V1_S_DEV - S_DEV, 266);
    assert_eq!(S_SUPER - V1_S_SUPER, 266);
    assert_eq!(share(S_DEV), 618_000);
    assert_eq!(share(S_SUPER), 293_000);
    println!("per 10,000 of an executed spin: prize {S_PRIZE}, jackpot {S_JACK}, mini {S_MINI}, super {S_SUPER}, seed {S_SEED}, administrator {S_DEV}");
    for (name, s) in [("administrator", S_DEV), ("super jackpot", S_SUPER), ("jackpot", S_JACK), ("mini", S_MINI), ("jackpot seed", S_SEED), ("all pots", S_JACK + S_MINI + S_SUPER + S_SEED), ("prize bank", S_PRIZE)] {
        let (n, d) = per_paid(s);
        println!("  {name:14} {s:5} bps of an executed spin = {:.3}% of paid bets ({}/{} bps)", n as f64 / d as f64 / 100.0, n / gcd(n, d), d / gcd(n, d));
    }
    // the invite reward: half of the project's income from the invitee's bets
    println!("  invite reward (50% of the project's income): {:.4}% of the invitee's paid bets with repeats, {:.2}% without (v1: 5.00% / 4.42%)", 618.0 / 0.884 / 200.0, 6.18 / 2.0);
}
fn gcd(a: u128, b: u128) -> u128 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

// ---------------------------------------------------------------------------------------------------------------------------
// 2. Table prizes do not depend on the change
// ---------------------------------------------------------------------------------------------------------------------------

/// The table prizes, repeats, mini and jackpot of a round are a function of the round value and of the round's inputs. The shares
/// enter in two places only: the amounts added to the pools (the prize bank's 8,939 is unchanged) and the width of the super
/// jackpot sector = S_SUPER contribution ÷ super amount. With the super amount scaled by 293/27 the sector is the same number of
/// 2^-32 units, and then the settlement with the upgrade's economics equals the settlement with v1's, field for field, except
/// what goes to the administrator and to the super pot. "v1" here is tests/v1ref, proven against the live build in xver_tests.
#[test]
fn rounds_are_identical_to_v1_at_an_equal_super_chance() {
    let mut sm = Sm(0xEC0_2026);
    let (mut rounds, mut stakes, mut with_pot, mut with_cut) = (0u32, 0u64, 0u32, 0u32);
    for k in 0..60_000u64 {
        let mode = sm.below(MODES as u64) as u8;
        let n = if mode == 0 { [10u16, 37, 100, 250, 1000][sm.below(5) as usize] } else { [10u16, 25, 100][sm.below(3) as usize] };
        // v1 super amount: a multiple of 27 lamports, so that the scaled amount is exact
        let s1 = (2 * SOL / 27 + sm.below(40 * SOL / 27)) * 27;
        let s2 = s1 / 27 * 293;
        assert_eq!(pot_odds(share(293), s2), pot_odds(share(27), s1));
        let j = JP_MIN_POT + sm.below(10 * SOL);
        let bank_sol = [SOL / 3, 4 * SOL, 500 * SOL][sm.below(3) as usize];
        let mk = |ss: u64| {
            let (mut c, _) = bank(bank_sol, j, ss, 50 * SOL);
            // pots between "open" and the pay caps: an ordinary state
            c.jackpot_pool = j + j / 3;
            c.mini_pool = mini_start(j) + mini_start(j) / 5;
            c.super_pool = ss + ss / 7;
            c.seed_pool = j / 9;
            let mut p = Player::default();
            engine::init_player(&mut p, key(1), 255, 1, T0);
            p.balance = n as u64 * COST;
            engine::commit_mode(&mut c, &mut p, T0 + 1, k + 1, n, mode, false, key(9)).unwrap();
            (c, p)
        };
        let seed = sm.seed();
        let late = k % 11 == 0;
        let ((mut c1, mut p1), (mut c2, mut p2)) = (mk(s1), mk(s2));
        let o1 = v1ref::settle_v1(&mut c1, &mut p1, seed, T0 + 2, late);
        let o2 = if late { engine::settle_late(&mut c2, &mut p2, seed, T0 + 2).unwrap() } else { engine::settle(&mut c2, &mut p2, Some(seed), T0 + 2).unwrap() };
        if o1.super_hits > 0 {
            assert!(o2.super_hits > 0, "the same spin wins the super jackpot in both");
            continue; // after a super win the two pots are different amounts by design (no top-up in the upgrade)
        }
        if o1.mini_hits + o1.jp_hits > 0 {
            // The same spin wins the same pot in both; the AMOUNT differs by design since the pots restart on their own
            // (a fifth of the jackpot and a third of the mini stay in the pot), and so may everything after it in the round.
            assert!(o2.mini_hits + o2.jp_hits > 0, "round {k}: the same spin wins the jackpot or the mini in both");
            assert_eq!((o1.mini_hits > 0, o1.jp_hits > 0), (o2.mini_hits > 0, o2.jp_hits > 0), "round {k}");
            with_pot += 1;
            continue;
        }
        assert_eq!(
            (o1.spins, o1.extras, o1.gold, o1.prizes, o1.best_prize, o1.mini_hits, o1.mini_won, o1.jp_hits, o1.jp_pool_total, o1.jp_to_player_now, o1.super_hits),
            (o2.spins, o2.extras, o2.gold, o2.prizes, o2.best_prize, o2.mini_hits, o2.mini_won, o2.jp_hits, o2.jp_pool_total, o2.jp_to_player_now, o2.super_hits),
            "round {k}"
        );
        assert_eq!((p1.balance, p1.jp_locked, p1.jp_per_week, p1.won_total), (p2.balance, p2.jp_locked, p2.jp_per_week, p2.won_total));
        assert_eq!((c1.prize_pool, c1.jackpot_pool, c1.mini_pool, c1.seed_pool, c1.seed_reserve, c1.promo_pool), (c2.prize_pool, c2.jackpot_pool, c2.mini_pool, c2.seed_pool, c2.seed_reserve, c2.promo_pool));
        // the only differences: 266,000 lamports per executed spin moved from the administrator to the super pot
        let shared = o1.spins; // a late round carries the shares too, only the pots do not pay
        assert_eq!(c1.dev_pool - c2.dev_pool, shared * 266_000);
        assert_eq!((c2.super_pool - (s2 + s2 / 7)) - (c1.super_pool - (s1 + s1 / 7)), shared * 266_000);
        rounds += 1;
        stakes += o1.spins;
        assert_eq!(o2.bank_released, 0, "no bank in this test is above BANK_KEEP");
        with_cut += (bank_sol < 4 * SOL && o1.prizes > 0) as u32;
    }
    println!("{rounds} rounds, {stakes} executed spins settled with the v1 and with the upgrade economics at an equal super chance: identical ({with_cut} with prizes from a small bank); {with_pot} more rounds had a mini or jackpot win: the same spin wins in both, the amount differs (kept shares)");
    assert!(rounds > 58_000 && with_pot > 500 && with_cut > 5_000);
}

/// With the SAME super amount (the administrator has not raised it) the super sector is wider by d = (293,000 − 27,000) ÷ amount.
/// For the same round number that moves FIVE narrow bands (independent verification of 18.11.2026; the first version of this
/// test looked at the wheel only and counted two):
///  * on the main wheel the two boundaries behind the gold sector (gold | repeat | nothing) move up by d: a draw in one of the
///    two bands of width d changes (repeat → gold, nothing → repeat);
///  * on the gold drum (a spin that is gold in both) the sectors are super | jackpot | mini | table prize, as shares of the
///    gold sector, and all three inner boundaries move: jackpot → super, mini → jackpot, TABLE PRIZE → MINI. Each band is
///    d × (gold sector behind the boundary ÷ gold sector) ≈ d of all spins, a little less.
/// Outside the five bands the outcome is the same; a table prize row depends on a separate number (u2) and is the same.
/// Total: just under 5 × d of all spins (0.0066% at 20 SOL, 0.0004% at 320 SOL); the prize chance and the table are unchanged.
#[test]
fn at_the_same_super_amount_five_narrow_bands_move() {
    let mut sm = Sm(77);
    let class = |g: Gold| match g {
        Gold::Super => 0u8,
        Gold::Jackpot => 1,
        Gold::Mini => 2,
        Gold::Prize(_) => 3,
    };
    for s in [SUPER_MIN_POT, 20 * SOL, 330 * SOL, START_MAX] {
        for mode in 0..MODES as u8 {
            let o1 = OddsBase::new_mode_share(5 * SOL, s, mode, V1_S_SUPER).at_super(u64::MAX / 4, u64::MAX / 4, u64::MAX / 4, true);
            let o2 = OddsBase::new_mode(5 * SOL, s, mode).at_super(u64::MAX / 4, u64::MAX / 4, u64::MAX / 4, true);
            assert_eq!((o1.jackpot, o1.mini, o1.prize, o1.again), (o2.jackpot, o2.mini, o2.prize, o2.again));
            let d = o2.superj - o1.superj;
            let (g1, a) = (o1.gold(), o1.again);
            assert_eq!(o2.gold(), g1 + d);
            // ---- the main wheel: exhaustive around the boundaries, random elsewhere
            let mut us: Vec<u64> = (0..200_000).map(|_| sm.next() & 0xFFFF_FFFF).collect();
            for b in [g1, g1 + d, g1 + a, g1 + a + d] {
                for x in b.saturating_sub(3)..=b + 3 {
                    us.push(x);
                }
            }
            for u in us {
                let in_band = (u >= g1 && u < g1 + d) || (u >= g1 + a && u < g1 + a + d);
                assert_eq!(draw_main(u as u32, &o1) != draw_main(u as u32, &o2), in_band, "u = {u}");
            }
            // ---- the gold drum. A sector boundary, as a count of u1 values below it: u1 × gold < c × 2^64 ⇔ u1 < ⌈c × 2^64 ÷ gold⌉
            let cut = |c: u64, gold: u64| -> u128 { ((c as u128) << 64).div_ceil(gold as u128) };
            let t1 = [cut(o1.superj, g1), cut(o1.superj + o1.jackpot, g1), cut(o1.superj + o1.jackpot + o1.mini, g1)];
            let t2 = [cut(o2.superj, g1 + d), cut(o2.superj + o2.jackpot, g1 + d), cut(o2.superj + o2.jackpot + o2.mini, g1 + d)];
            let by_cuts = |u1: u64, t: &[u128; 3]| t.iter().filter(|x| u1 as u128 >= **x).count() as u8;
            // every boundary moved up, and (unless d is larger than a whole neighbouring sector) by less than a sector
            assert!(t2[0] > t1[0] && t2[1] > t1[1] && t2[2] > t1[2]);
            let mut u1s: Vec<u64> = (0..20_000).map(|_| sm.next()).collect();
            for t in t1.iter().chain(t2.iter()) {
                for x in (*t as u64).saturating_sub(2)..=(*t as u64) + 2 {
                    u1s.push(x);
                }
                u1s.push(((*t as u64) / 2).max(1));
            }
            for k in 0..3 {
                u1s.push(((t1[k] + t2[k]) / 2) as u64); // the middle of each band
            }
            for u1 in u1s {
                let u2 = sm.next() as u32;
                let (c1, c2) = (draw_gold_mode(u1, u2, &o1, mode), draw_gold_mode(u1, u2, &o2, mode));
                // the program's draw is exactly the three cuts
                assert_eq!((class(c1), class(c2)), (by_cuts(u1, &t1), by_cuts(u1, &t2)), "u1 = {u1}");
                if let (Gold::Prize(a), Gold::Prize(b)) = (c1, c2) {
                    assert_eq!(a, b, "the table prize row does not depend on the shares");
                }
            }
            // the three bands, exactly: the u1 values whose sector differs = between the old and the new position of a boundary
            let mut cuts: Vec<u128> = t1.iter().chain(t2.iter()).copied().collect();
            cuts.push(0);
            cuts.push(1u128 << 64);
            cuts.sort();
            let mut moved_u1: u128 = 0;
            for w in cuts.windows(2) {
                if w[1] > w[0] && by_cuts(w[0] as u64, &t1) != by_cuts(w[0] as u64, &t2) {
                    moved_u1 += w[1] - w[0];
                }
            }
            // as a share of ALL spins: a spin is gold under both rules when u < g1; then the band applies
            let gold_moved = g1 as f64 * (moved_u1 as f64 / 18_446_744_073_709_551_616.0); // in 2^-32 units, like d
            let total = 2.0 * d as f64 + gold_moved;
            // three bands of just under d each. Only when d is wider than a whole neighbouring sector (the smallest super amounts:
            // at 2 SOL d is 7.6 jackpot sectors) the bands run into each other and their union is smaller: a draw then jumps
            // two sectors (table prize → jackpot), still inside the same stretch of the drum.
            let apart = d < o1.jackpot.min(o1.mini);
            assert!(gold_moved <= 3.0 * d as f64 && gold_moved > if apart { 2.9 } else { 0.99 } * d as f64, "mode {mode}, super {s}: gold drum bands {gold_moved} vs d {d}");
            assert!(total <= 5.0 * d as f64 && total > if apart { 4.9 } else { 2.99 } * d as f64);
            assert_eq!(apart, s >= 20 * SOL, "from 20 SOL up the five bands are separate");
            // the third band is "table prize → mini" (when d is smaller than the mini sector the bands do not overlap)
            if d < o1.mini {
                let mid = ((t1[2] + t2[2]) / 2) as u64;
                assert!(matches!(draw_gold_mode(mid, 0, &o1, mode), Gold::Prize(_)) && draw_gold_mode(mid, 0, &o2, mode) == Gold::Mini);
                let mid = ((t1[1] + t2[1]) / 2) as u64;
                assert!(draw_gold_mode(mid, 0, &o1, mode) == Gold::Mini && draw_gold_mode(mid, 0, &o2, mode) == Gold::Jackpot);
            }
            if d < o1.jackpot {
                let mid = ((t1[0] + t2[0]) / 2) as u64;
                assert!(draw_gold_mode(mid, 0, &o1, mode) == Gold::Jackpot && draw_gold_mode(mid, 0, &o2, mode) == Gold::Super);
            }
            if mode == 0 {
                println!(
                    "super amount {:>5} SOL: wheel 2 bands of {} of 2^32 = {:.6}% of spins each; gold drum 3 bands, together {:.6}% of spins; all five {:.6}% of spins (v1 super chance {:.6}%, upgrade {:.6}%)",
                    s / SOL, d, d as f64 / Q32 as f64 * 100.0, gold_moved / Q32 as f64 * 100.0, total / Q32 as f64 * 100.0, o1.superj as f64 / Q32 as f64 * 100.0, o2.superj as f64 / Q32 as f64 * 100.0
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------
// 3. No state of the pots makes a spin worth its stake (K2)
// ---------------------------------------------------------------------------------------------------------------------------

/// Exact expected value of one executed spin, in lamports × 2^32, for the given chances and pot payouts (the table prizes at
/// their full amounts — the cap by the bank can only lower them), NOT counting the repeat.
fn spin_ev_q32(o: &Odds, mode: u8, jackpot_win: u64, mini_win: u64, super_win: u64) -> u128 {
    let lad = &MODE_LADDER[mode as usize];
    // a table prize: chance o.prize / 2^32; row i with the exact weight the program gives it (floor(u2 × 10,000 / 2^32) falls in the row)
    let mut row_sum: u128 = 0; // Σ rows(count of u2 values) × lamports
    let mut c = 0u64;
    let mut prev: u128 = 0;
    for (tok, w) in lad.iter() {
        c += w;
        let upto = ((c as u128) << 32).div_ceil(LADDER_SUM as u128).min(Q32); // number of u2 with (u2 × 10,000) >> 32 < c
        row_sum += (upto - prev) * (*tok * ONE) as u128;
        prev = upto;
    }
    assert_eq!(prev, Q32);
    // row_sum / 2^32 = expected table prize; multiply the chance in q32 units: the result is lamports × 2^32, rounded UP
    (o.prize as u128 * row_sum).div_ceil(Q32) + o.jackpot as u128 * jackpot_win as u128 + o.mini as u128 * mini_win as u128 + o.superj as u128 * super_win as u128
}

/// The proof, with the program's own chances (in 2^-32 units) and the largest amounts a pot can pay:
///   V = value of one executed spin ≤ E[table] + E[pots] + P(repeat) × V   ⇒   V ≤ (E[table] + E[pots]) ÷ (1 − P(repeat)).
/// The largest pot payouts: the winner's share of at most ×3 (jackpot, mini) / ×2.5 (super) of the amount recorded at bet time —
/// whatever the pots hold (the pot ÷ amount ratio itself is unbounded: a pot that is not won keeps growing).
/// Checked for every game, for starting amounts from the minimum to the maximum.
#[test]
fn no_state_of_the_pots_makes_a_spin_worth_its_stake() {
    let mut worst = (0u128, 1u128, 0u8, 0u64, 0u64);
    let amounts = [0u64, JP_MIN_POT, 777_777_777, 5 * SOL, 20 * SOL, 333 * SOL, START_MAX];
    for mode in 0..MODES as u8 {
        for &j in &amounts {
            for &s in &amounts {
                let ob = OddsBase::new_mode(j, s, mode);
                let o = ob.at_super(u64::MAX / 4, u64::MAX / 4, u64::MAX / 4, true); // every pot open
                let jw = (pay_cap(jackpot_start(j), JP_PAY_CAP_BPS) as u128 * JP_WINNER_BPS as u128 / BPS as u128) as u64;
                let mw = pay_cap(mini_start(j), MINI_PAY_CAP_BPS);
                let sw = (pay_cap(super_start(s), SUPER_PAY_CAP_BPS) as u128 * SUPER_WINNER_BPS as u128 / BPS as u128) as u64;
                let ev = spin_ev_q32(&o, mode, jw, mw, sw);
                let den = Q32 - o.again as u128; // (1 − P(repeat)) × 2^32
                // V = ev / den lamports; V < COST ?
                assert!(ev < COST as u128 * den, "mode {mode}, jackpot {j}, super {s}: a spin is worth {} of the stake", ev as f64 / den as f64 / COST as f64);
                if ev * worst.1 > worst.0 * den {
                    worst = (ev, den, mode, j, s);
                }
            }
        }
    }
    let v = worst.0 as f64 / worst.1 as f64 / COST as f64;
    println!("worst state (every pot at or above its pay cap; game {}, jackpot amount {} SOL, super amount {} SOL): one bet returns {:.4}% in expectation", worst.2, worst.3 as f64 / SOL as f64, worst.4 as f64 / SOL as f64, v * 100.0);
    assert!(v < 0.9950);
    // the same from the constants alone (the compile-time assert in consts.rs): (table + pots) ÷ (1 − 0.116)
    let table = mode_ev_ppm_tokens(0) as f64 / 1e6; // points per spin; a bet = 10,000 points
    let pots = pots_ev_bound_scaled() as f64 / 1e6 / (BPS * BPS) as f64;
    println!("bound from the constants: table {:.3} + pots {:.2} = {:.2} of 10,000 per executed spin; ÷ 0.884 = {:.3}% of a bet (v1 at pots = starting amounts: {:.2}%; v1 had no bound)", table, pots, table + pots, (table + pots) / 0.884 / 100.0, (table + 88.0 * 0.95 + 44.0 + 27.0 * 0.9) / 0.884 / 100.0);
    // the typical states for comparison: pots at one starting amount
    for (name, r) in [("pots at 1 starting amount", 1.0), ("pots at 2", 2.0)] {
        println!("  {name}: {:.2}% of a bet", (table + r * (88.0 * 0.95 + 44.0 + 293.0 * 0.9)) / 0.884 / 100.0);
    }
    // without the caps: where the super pot alone would make a bet worth more than it costs (jackpot and mini at 1 amount)
    let head = 8_840.0 - table - 88.0 * 0.95 - 44.0;
    println!("  without a cap a bet would be worth more than it costs from super pot ÷ amount > {:.2} (v1, 27 bps: > {:.1})", head / (293.0 * 0.9), head / (27.0 * 0.9));
}

/// The same measured with the engine: a player who bets ONLY when every pot holds a thousand starting amounts (the best state
/// there can be). The measured return must agree with the exact expectation above (99.458% at the smallest amounts); the
/// statistical error of 60 million bets is about ±0.13% (one standard deviation), so the measurement confirms the arithmetic —
/// the proof that the expectation is below 100% is the exact computation, not this sample.
#[test]
fn engine_in_the_richest_state_matches_the_exact_expectation() {
    let bets: u64 = std::env::var("FD_ECON_RICH_BETS").ok().and_then(|s| s.parse().ok()).unwrap_or(60_000_000);
    let seed0: u64 = std::env::var("FD_ECON_RICH_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(4242);
    let (mut c, _) = bank(1_000_000 * SOL, JP_MIN_POT, SUPER_MIN_POT, 0);
    let mut p = Player::default();
    engine::init_player(&mut p, key(1), 255, 1, T0);
    let mut sm = Sm(seed0);
    let (mut paid, mut got, mut exec, mut table) = (0u128, 0u128, 0u64, 0u128);
    for k in 0..bets / 1_000 {
        c.jackpot_pool = 1_000 * JP_MIN_POT;
        c.mini_pool = 1_000 * MINI_MIN_POT;
        c.super_pool = 1_000 * SUPER_MIN_POT;
        c.prize_pool = 1_000_000 * SOL;
        p.balance = 1_000 * COST;
        p.jp_locked = 0;
        p.jp_per_week = 0;
        engine::commit(&mut c, &mut p, T0 + 1, k + 1, 1_000, false, key(9)).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(sm.seed()), T0 + 2).unwrap();
        paid += 1_000 * COST as u128;
        got += p.balance as u128 + p.jp_locked as u128;
        exec += out.spins;
        table += out.prizes as u128;
        p.balance = 0;
    }
    let o = OddsBase::new_mode(JP_MIN_POT, SUPER_MIN_POT, 0).at_super(u64::MAX / 4, u64::MAX / 4, u64::MAX / 4, true);
    // jackpot: 95% of the four fifths that do not stay; mini: what does not stay of ×3
    let (jw, mw, sw) = (JP_MIN_POT * 3 / 100 * 80 / 100 * 95, MINI_MIN_POT * 3 - MINI_MIN_POT * 3 * MINI_KEEP_BPS / BPS, SUPER_MIN_POT / 2 * 5 / 10 * 9);
    let exact = spin_ev_q32(&o, 0, jw, mw, sw) as f64 / (Q32 - o.again as u128) as f64 / COST as f64;
    let rtp = got as f64 / paid as f64;
    // every pot win paid exactly the capped amount, whatever the pot held
    assert_eq!(got, table + c.jackpots_won as u128 * jw as u128 + c.minis_won as u128 * mw as u128 + c.supers_won as u128 * sw as u128);
    let expect = |odds: u64| exec as f64 * odds as f64 / Q32 as f64;
    println!(
        "{bets} bets placed only in the richest state: returned {:.3}% (exact expectation {:.3}%); super jackpots {} of 4.5 SOL (expected {:.0}), jackpots {} (expected {:.0}), minis {} (expected {:.0}); table prizes {:.3}% of the bets (expected {:.3}%)",
        rtp * 100.0, exact * 100.0, c.supers_won, expect(o.superj), c.jackpots_won, expect(o.jackpot), c.minis_won, expect(o.mini), table as f64 / paid as f64 * 100.0, mode_ev_ppm_tokens(0) as f64 / 1e6 / 0.884 / 100.0
    );
    assert!((rtp - exact).abs() < 0.006, "measured {rtp}, exact {exact}");
    for (won, odds) in [(c.supers_won, o.superj), (c.jackpots_won, o.jackpot), (c.minis_won, o.mini)] {
        let e = expect(odds);
        assert!((won as f64 - e).abs() < 4.0 * e.sqrt(), "pot wins {won}, expected {e}");
    }
}

/// The bound above is for the PROGRAM. Two off-chain flows return part of the project's share to a player and must be added:
///  * the invite reward (scripts/referrals.ts, REF_SHARE_BPS = 5,000: half of the project's income from the invitee's bets, paid by
///    hand): a player who invites himself with a second wallet gets it back on his own bets, if the owner pays it;
///  * the administrator playing himself gets his whole share back.
/// Everything per executed spin, in 1/10,000 of a bet ("points"); a paid bet is 1/0.884 executed spins, so the same ratios hold
/// per paid bet. Return = (table + pots + returned part of 618) ÷ 8,840.
#[test]
fn invite_kickback_and_administrator_play() {
    let table = mode_ev_ppm_tokens(0) as f64 / 1e6; // 7,750.062
    let head = (BPS - W_AGAIN) as f64; // 8,840
    // since the pots restart on their own a fifth of the jackpot and a third of the mini stay in the pot (JP_KEEP_BPS, MINI_KEEP_BPS)
    let (kj, km) = (1.0 - JP_KEEP_BPS as f64 / BPS as f64, 1.0 - MINI_KEEP_BPS as f64 / BPS as f64);
    let pots_at = |rj: f64, rm: f64, rs: f64| 88.0 * 0.95 * kj * rj + 44.0 * km * rm + S_SUPER as f64 * 0.9 * rs;
    let pmax = pots_ev_kept_bound_scaled() as f64 / 1e6 / (BPS * BPS * BPS) as f64; // 947.894: every pot at its pay cap (1,042.05 before the kept shares)
    let ret = |pots: f64, back: f64| (table + pots + back) / head * 100.0;
    // exact: the reward of a self-inviting player, incl. free repeats = 618 × 50% ÷ 0.884 bps of his paid bets
    let (n, d) = (S_DEV as u128 * 5_000 * BPS as u128, (BPS - W_AGAIN) as u128 * BPS as u128);
    assert_eq!((n * 1_000_000 / d, S_DEV * 5_000 / BPS), (349_547_511, 309), "349.5475 bps = 3.495% of paid bets (38625/11050 %); without repeats 3.09%");
    let kick = S_DEV as f64 * 0.5;
    println!("self-inviting player (reward 50% of the project's share = {:.4}% of his paid bets, paid):", kick / head * 100.0);
    println!("  pots at 1 starting amount {:.2}% · the long-run average (all pot money paid out) {:.2}% · pots at 2 {:.2}% · every pot at its pay cap {:.2}%", ret(pots_at(1.0, 1.0, 1.0), kick), ret(443.0 - 0.05 * 106.0 - 0.0, kick), ret(pots_at(2.0, 2.0, 2.0), kick), ret(pmax, kick));
    assert!(ret(pots_at(1.0, 1.0, 1.0), kick) < 96.0 && ret(pmax, kick) > 100.0, "with the reward paid the worst case is above 100%");
    // from which super pot the self-inviter's bet is worth more than it costs (jackpot and mini at 1.25 amounts: reserve empty)
    let need = head - table - kick;
    let rs = (need - pots_at(1.25, 1.25, 0.0)) / (S_SUPER as f64 * 0.9);
    println!("  above 100% when the pots are worth more than {:.1} of 10,000 per spin (average ≈ 437): with jackpot and mini at 1.25 amounts — super pot above {:.2} targets (80 SOL target: {:.0} SOL; 320: {:.0} SOL; 1000: {:.0} SOL)", need, rs, rs * 80.0, rs * 320.0, rs * 1000.0);
    assert!(rs > 2.45 && rs < 2.55);
    // ONE basis for both thresholds (independent verification of 18.11.2026: the two lines of the table used different ones).
    // Super pot, in targets, from which a bet is worth more than it costs, with the jackpot and the mini at 1 / 1.25 / their caps:
    let from = |back: f64, kjm: f64| (head - table - back - pots_at(kjm, kjm, 0.0)) / (S_SUPER as f64 * 0.9);
    let cap_jm = JP_PAY_CAP_BPS as f64 / BPS as f64;
    println!("  above 100% from a super pot of (targets), jackpot and mini at 1 amount / 1.25 / their caps: self-inviter with the reward {:.2} / {:.2} / {:.2}; the administrator's own play {:.2} / {:.2} / {:.2}", from(kick, 1.0), from(kick, 1.25), from(kick, cap_jm), from(S_DEV as f64, 1.0), from(S_DEV as f64, 1.25), from(S_DEV as f64, cap_jm));
    for (got, want) in [(from(kick, 1.0), 2.60), (from(kick, 1.25), 2.505), (from(kick, cap_jm), 1.867), (from(S_DEV as f64, 1.0), 1.425), (from(S_DEV as f64, 1.25), 1.334), (from(S_DEV as f64, cap_jm), 0.695)] {
        assert!((got - want).abs() < 0.006, "{got} vs {want}");
    }
    // the largest invite share that keeps the worst case below 100% with the pay caps as built
    let max_bps = ((head - table - pmax) / S_DEV as f64 * BPS as f64).floor() as u64;
    assert_eq!(max_bps, 2_298);
    assert!(ret(pmax, S_DEV as f64 * 2_298.0 / 1e4) < 100.0 && ret(pmax, S_DEV as f64 * 2_299.0 / 1e4) >= 100.0);
    println!("  largest invite share with the worst case below 100%: {max_bps} bps of the project's share (22.98%; 7.74% before the kept shares); at 5,000 bps the worst case is {:.2}%", ret(pmax, kick));
    // with every cap at its lowest allowed value (×2) a 50% reward would fit since the kept shares (it did not before: 4,973 bps)
    let max_at_2 = ((head - table - pots_at(2.0, 2.0, 2.0)) / S_DEV as f64 * BPS as f64).floor() as u64;
    println!("  with all three caps at ×2 (the lowest the starting-amount rules allow) the largest share would be {max_at_2} bps");
    assert!(max_at_2 > 5_900 && max_at_2 < 6_000);
    // the administrator's own play: the whole 618 comes back
    let dev = S_DEV as f64;
    let v1 = (table + 88.0 * 0.95 + 44.0 + 27.0 * 0.9 + 884.0) / head * 100.0;
    let rs_admin = (head - table - dev - pots_at(1.0, 1.0, 0.0)) / (S_SUPER as f64 * 0.9);
    let rs_admin_v1 = (head - table - 884.0 - (88.0 * 0.95 + 44.0)) / (27.0 * 0.9);
    println!("administrator's own play (his whole share returns): pots at 1 amount {:.2}% (v1: {:.2}%) · every pot at its cap {:.2}%; above 100% from super pot > {:.2} targets with jackpot and mini at 1 amount (v1: > {:.2} starting amounts)", ret(pots_at(1.0, 1.0, 1.0), dev), v1, ret(pmax, dev), rs_admin, rs_admin_v1);
    assert!((v1 - 99.39).abs() < 0.01 && (ret(pots_at(1.0, 1.0, 1.0), dev) - 98.73).abs() < 0.01 && (rs_admin - 1.42).abs() < 0.01 && (ret(pmax, dev) - 105.38).abs() < 0.01);
}

// ---------------------------------------------------------------------------------------------------------------------------
// 4. The super jackpot: how often, how large (for the owner's choice of the amount)
// ---------------------------------------------------------------------------------------------------------------------------

/// One line of the table for a super amount `s` (lamports): a million win-to-win cycles with the program's own chance per spin.
/// After a win the pot keeps 10% (and whatever was above ×2.5 of the amount), accumulates to a quarter of the amount while it
/// cannot be won, and is then won with the constant chance. Jackpot wins add 5% of the jackpot: on average 5,300 lamports per spin
/// (5% of the jackpot's 88 + 18 bps) when the reserve is empty.
fn super_cycles(s: u64, sm: &mut Sm) -> (f64, f64, f64, f64, f64, f64) {
    let per_spin = share(S_SUPER) as f64 + 0.05 * (share(S_JACK) + share(S_SEED)) as f64;
    let p = pot_odds(share(S_SUPER), s) as f64 / Q32 as f64;
    let (open, cap) = (open_pot(s) as f64, pay_cap(s, SUPER_PAY_CAP_BPS) as f64);
    let mut pot = 0.1 * 1.25 * s as f64;
    let n = 1_000_000;
    let (mut spins, mut won, mut pots, mut capped, mut closed) = (0f64, 0f64, 0f64, 0u32, 0f64);
    let mut wins: Vec<f64> = Vec::with_capacity(n);
    for _ in 0..n {
        let wait_closed = ((open - pot).max(0.0) / per_spin).ceil();
        let t = (sm.unit().ln() / (1.0 - p).ln()).floor() + 1.0;
        pot += (wait_closed + t) * per_spin;
        let win = 0.9 * pot.min(cap);
        capped += (pot > cap) as u32;
        pots += pot;
        pot -= win;
        spins += wait_closed + t;
        closed += wait_closed;
        won += win;
        wins.push(win);
    }
    wins.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = n as f64;
    (spins / n, won / n, pots / n, capped as f64 / n, closed / spins, wins[wins.len() / 2])
}

#[test]
fn super_jackpot_frequency_and_size_by_amount() {
    let mut sm = Sm(2026_11_18);
    let paid_per_spin = (BPS - W_AGAIN) as f64 / BPS as f64 * 0.01; // SOL of paid bets per executed spin
    let per_spin = share(S_SUPER) as f64 + 0.05 * (share(S_JACK) + share(S_SEED)) as f64;
    println!("growth of the super pot: {:.4} SOL per 1 SOL of paid bets ({} + ≈5,300 lamports per executed spin, 1.1312 executed spins per bet)", per_spin / 1e9 / paid_per_spin, share(S_SUPER));
    println!("amount | chance per 0.1 SOL round (1 in) | wagered between wins | pot at a win, avg | winner gets avg / median | wins from more than ×2.5 | share of time not winnable");
    for s_sol in [20u64, 80, 320, 1_000] {
        let s = s_sol * SOL;
        let (spins, won, pot, capped, closed, median) = super_cycles(s, &mut sm);
        let p_spin = pot_odds(share(S_SUPER), s) as f64 / Q32 as f64;
        let p_round = 1.0 - (1.0 - p_spin).powf(10.0 / 0.884);
        println!(
            "{:>5} SOL | {:>9.0} | {:>7.0} SOL | {:>6.0} SOL | {:>5.0} / {:>5.0} SOL | {:>4.1}% | {:>4.1}%",
            s_sol,
            1.0 / p_round,
            spins * paid_per_spin,
            pot / 1e9,
            won / 1e9,
            median / 1e9,
            capped * 100.0,
            closed * 100.0
        );
        // sanity of the model against the closed form: the open phase lasts amount ÷ contribution spins on average
        assert!((spins * (1.0 - closed) * p_spin - 1.0).abs() < 0.01);
        if s_sol == 320 {
            // the recommended amount (two ×4 steps from 20 SOL): one win per ≈11 thousand SOL of bets, the winner gets ≈365 SOL on average
            assert!(spins * paid_per_spin > 10_000.0 && spins * paid_per_spin < 12_000.0 && won / 1e9 > 330.0 && won / 1e9 < 400.0, "{} {}", spins * paid_per_spin, won / 1e9);
        }
        // what the pot looks like after V SOL of bets if it starts at 12.5% of the amount and is not won, and how likely that is
        let mut line = String::new();
        for v in [1_000.0f64, 10_000.0, 100_000.0] {
            let n = v / paid_per_spin;
            let start = 0.125 * s as f64;
            let potv = start + n * per_spin;
            let closed_spins = ((0.25 * s as f64 - start) / per_spin).min(n);
            let alive = (1.0 - p_spin).powf(n - closed_spins);
            line += &format!(" after {:>6.0} SOL of bets: {:>6.0} SOL (not won yet with probability {:>5.1}%);", v, potv / 1e9, alive * 100.0);
        }
        println!("        from {:.1} SOL:{}", 0.125 * s_sol as f64, line);
    }
    println!("v1 for comparison (27 bps, 20 SOL, topped up from the reserve): 1 win per {:.0} SOL of bets, the winner gets ≈{:.0} SOL, of which ≈{:.0} SOL is the administrator's top-up", 20e9 / 27_000.0 * paid_per_spin, 0.9 * 2.0 * 20.0 * 1.2, 0.8 * 20.0);
}

// ---------------------------------------------------------------------------------------------------------------------------
// 5. Long simulation
// ---------------------------------------------------------------------------------------------------------------------------

/// 12 million paid bets (120,000 SOL) of 40 players over ≈ 400 days: every game, rounds of 10…1000 bets, deposits, withdrawals,
/// late settlements and refunds, the administrator raises the super amount 20 → 80 → 320 SOL by the two allowed ×4 steps, collects
/// the income, and NEVER donates. Checked after every operation: the vault equals pools + obligations to the lamport.
#[test]
fn long_simulation_twelve_million_bets() {
    let target: u64 = std::env::var("FD_ECON_STAKES").ok().and_then(|s| s.parse().ok()).unwrap_or(12_000_000);
    let (mut c, mut vault) = bank(200 * SOL, 5 * SOL, 20 * SOL, 30 * SOL);
    let funded = vault as u128;
    let mut sm = Sm(0x5EED_2026_1118);
    let np = 40usize;
    let mut ps: Vec<Player> = (0..np as u64)
        .map(|i| {
            let mut p = Player::default();
            let n = engine::assign_number(&mut c).unwrap();
            engine::init_player(&mut p, key(i), 255, n, T0);
            p
        })
        .collect();
    let mut owed_total: u128 = 0;
    let mut now = T0 + 10;
    let secs_per_round = 400.0 * 86_400.0 / (target as f64 / 110.0);
    let mut clock = now as f64;
    let (mut staked, mut exec, mut exec_shared, mut deposited, mut withdrawn, mut claimed) = (0u128, 0u128, 0u128, 0u128, 0u128, 0u128);
    let (mut refunds, mut lates, mut rounds) = (0u64, 0u64, 0u64);
    let (mut super_paid, mut super_max, mut super_min_pool) = (0u128, 0u64, u64::MAX);
    let (mut closed_super, mut closed_jack, mut closed_mini) = (0u64, 0u64, 0u64);
    let mut reserve_zero_since: Option<u64> = None;
    let ladder = [(80 * SOL, 1u64), (320 * SOL, 2)];
    let mut step = 0usize;
    let mut quarter_wins = [[0u64; 3]; 4];
    let mut super_log: Vec<(u128, u64)> = Vec::new();
    let mut last_super_pool = c.super_pool;
    let mut paid_stakes: u64 = 0;
    while paid_stakes < target {
        rounds += 1;
        clock += secs_per_round;
        now = clock as i64;
        // the administrator: the two allowed raises of the super amount, one per day at the start; a claim now and then
        if step < ladder.len() && now >= T0 + ladder[step].1 as i64 * (STARTS_DELAY_SECS + 3_600) {
            engine::set_starts(&mut c, 5 * SOL, ladder[step].0, now).expect("a raise within ×4 of the amount in force");
            step += 1;
        }
        if rounds % 5_000 == 0 && c.dev_pool > 0 {
            claimed += c.dev_pool as u128;
            vault -= c.dev_pool;
            c.dev_pool = 0;
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
        let (jw, mw, sw) = (c.jackpots_won, c.minis_won, c.supers_won);
        let pool_before = c.super_pool;
        if fate == 0 {
            engine::refund(&mut c, p, now + 600).unwrap();
            refunds += 1;
        } else {
            let late = fate < 4;
            let seed = sm.seed();
            let out = if late { engine::settle_late(&mut c, p, seed, now + 2).unwrap() } else { engine::settle(&mut c, p, Some(seed), now + 2).unwrap() };
            lates += late as u64;
            staked += stake as u128;
            paid_stakes += n as u64;
            exec += out.spins as u128;
            exec_shared += out.spins as u128;
            if out.super_hits > 0 {
                super_paid += out.super_total as u128;
                super_max = super_max.max(out.super_total);
                super_log.push((staked, out.super_total));
                assert!(out.super_total as u128 <= pay_cap(p.p_start_super, SUPER_PAY_CAP_BPS) as u128 * out.super_hits as u128 * 9 / 10 + 1);
                assert!(c.super_pool >= (pool_before - out.super_total.min(pool_before)) / 2, "the pot keeps at least 10% after a win");
            } else {
                assert!(c.super_pool >= pool_before + out.spins * share(S_SUPER), "the super pot only grows between wins");
            }
        }
        let q = ((paid_stakes as u128 * 4 / target as u128).min(3)) as usize;
        quarter_wins[q][0] += c.minis_won - mw;
        quarter_wins[q][1] += c.jackpots_won - jw;
        quarter_wins[q][2] += c.supers_won - sw;
        super_min_pool = super_min_pool.min(c.super_pool);
        last_super_pool = c.super_pool;
        closed_super += (c.super_pool < open_pot(super_start(c.init_super))) as u64;
        closed_jack += (c.jackpot_pool < open_pot(jackpot_start(c.init_jackpot))) as u64;
        closed_mini += (c.mini_pool < open_pot(mini_start(c.init_jackpot))) as u64;
        if c.seed_reserve == 0 && reserve_zero_since.is_none() {
            reserve_zero_since = Some(paid_stakes);
        }
        let p = &mut ps[i];
        // a withdrawal now and then (everything that can be withdrawn)
        if sm.below(40) == 0 {
            let can = engine::withdrawable(p, now + 3);
            if can > 0 {
                engine::withdraw(&mut c, p, can, now + 3).unwrap();
                vault -= can;
                withdrawn += can as u128;
            }
        }
        owed_total = owed_total + owed(p) - b;
        // the solvency invariant of lib.rs (`solvent`), here as an equality: nothing appears and nothing disappears
        assert_eq!(vault as u128, pools(&c) + owed_total, "round {rounds}");
    }
    let _ = last_super_pool;
    // ---- the project's income is the new share, to the lamport ----
    assert_eq!(c.dev_total as u128, exec_shared * share(S_DEV) as u128);
    assert_eq!(c.dev_pool as u128 + claimed, c.dev_total as u128);
    let dev_pct = pct(c.dev_total as u128, staked);
    // ---- everything that came in is either paid, owed, in the pools, or the administrator's ----
    assert_eq!(funded + deposited, withdrawn + claimed + pools(&c) + owed_total);
    let players_side = c.paid_total as u128;
    assert!(players_side <= funded + staked - c.dev_total as u128, "paid out more than was taken in");
    let owed_check: u128 = ps.iter().map(owed).sum();
    assert_eq!(owed_check, owed_total);
    let held = (c.prize_pool as u128 + c.jackpot_pool as u128 + c.mini_pool as u128 + c.super_pool as u128 + c.seed_pool as u128 + c.seed_reserve as u128) as f64 / 1e9;
    println!("paid bets {} = {:.0} SOL in {rounds} rounds (executed spins {exec}, {:.4} per bet); refunds {refunds}, late {lates}", paid_stakes, staked as f64 / 1e9, exec as f64 / paid_stakes as f64);
    println!("project income {:.3} SOL = {:.4}% of paid bets (618 bps of every executed spin, exact); credited to players {:.2}%; held for players in the bank and pots {:.0} SOL (it was {:.0})", c.dev_total as f64 / 1e9, dev_pct, pct(players_side, staked), held, funded as f64 / 1e9);
    println!("pot wins: mini {}, jackpot {}, super {}; by quarter of the run (mini, jackpot, super): {:?}", c.minis_won, c.jackpots_won, c.supers_won, quarter_wins);
    println!("super jackpot: paid {:.0} SOL in {} wins = {:.0} SOL on average, largest {:.0} SOL; pot now {:.0} SOL, smallest ever {:.1} SOL; one win per {:.0} SOL of bets", super_paid as f64 / 1e9, c.supers_won, super_paid as f64 / 1e9 / c.supers_won.max(1) as f64, super_max as f64 / 1e9, c.super_pool as f64 / 1e9, super_min_pool as f64 / 1e9, staked as f64 / 1e9 / c.supers_won.max(1) as f64);
    println!("super wins (SOL of bets so far → SOL to the winner): {}", super_log.iter().map(|(v, w)| format!("{:.0}→{:.0}", *v as f64 / 1e9, *w as f64 / 1e9)).collect::<Vec<_>>().join(", "));
    println!("seed reserve: 30 SOL at the start, no donations → {:.3} SOL; first empty after {:?} bets; share of rounds after which a pot could not be won: super {:.1}%, jackpot {:.1}%, mini {:.1}%", c.seed_reserve as f64 / 1e9, reserve_zero_since, closed_super as f64 / rounds as f64 * 100.0, closed_jack as f64 / rounds as f64 * 100.0, closed_mini as f64 / rounds as f64 * 100.0);
    if target >= 5_000_000 {
        assert!((dev_pct - 6.991).abs() < 0.01, "project income {dev_pct}% of paid bets");
        // nothing stalls: every pot is won in every quarter of the run (the super at least in three of four: it is rare by design)
        for q in 0..4 {
            assert!(quarter_wins[q][0] > 100 && quarter_wins[q][1] > 20, "quarter {q}: {:?}", quarter_wins[q]);
        }
        assert!(quarter_wins.iter().filter(|q| q[2] > 0).count() >= 3 && c.supers_won >= 6, "super wins by quarter {:?}", quarter_wins);
        let avg = super_paid as f64 / 1e9 / c.supers_won as f64;
        assert!(avg > 150.0 && avg < 600.0, "average super jackpot {avg}");
        assert!(closed_super as f64 / (rounds as f64) < 0.35 && closed_jack as f64 / (rounds as f64) < 0.30 && closed_mini as f64 / (rounds as f64) < 0.45);
        // return to players over the run: prizes + pots, between the floor (table only) and the long-run ceiling of 92.7%
        let rtp = pct(players_side, staked);
        assert!(rtp > 86.0 && rtp < 93.5, "credited to players {rtp}%");
    }
}

// ---------------------------------------------------------------------------------------------------------------------------
// 6. The invite reward and rich pots (off-chain rule of scripts/referrals.ts, owner's decision of 18.11.2026)
// ---------------------------------------------------------------------------------------------------------------------------
// The reward (50% of the project's 618 per executed spin) is paid by hand from the report. Passed back to the player it makes
// a bet in the richest states worth more than it costs (101.89% with the kept shares). The report therefore pays NO reward for a round
// whose pots were worth more than RICH_POT_LIMIT points per executed spin (exact coefficients of the final constants):
//     66.88·kj + 29.3348·km + 263.7·ks > 750,   k = min(pot seen by the round, pay cap) ÷ starting amount recorded at bet time.
// Everything is read from the RoundInputs event (engine::round_inputs). The same function is written twice, here and in
// scripts/referrals.ts (`richPot`), and compared on the vectors of tests/local/fixtures/rich-pot-vectors.json.

/// The limit of scripts/referrals.ts (RICH_POT_LIMIT), points of 10,000 per executed spin
const RICH_POT_LIMIT: u64 = 750;
/// The shipped invite share (REF_SHARE_BPS)
const INVITE_SHARE_BPS: u64 = 5_000;

/// The coefficients, from the constants: the pot's contribution × the winner's share of what leaves the pot (a fifth of
/// a jackpot win and a third of a mini win stay in it: JP_KEEP_BPS, MINI_KEEP_BPS), in points × BPS²
fn pot_coefficients() -> (u128, u128, u128) {
    ((S_JACK * JP_WINNER_BPS * (BPS - JP_KEEP_BPS)) as u128, (S_MINI * BPS * (BPS - MINI_KEEP_BPS)) as u128, (S_SUPER * SUPER_WINNER_BPS * BPS) as u128)
}
const BPS2: f64 = (BPS * BPS) as f64;

/// The pots a round can see (lamports), given what is known before its first spin; E = 2n + 16 is the largest number of executed
/// spins. Own contribution of every spin, plus the EXPECTED rise from pot restarts inside the round (see scripts/referrals.ts).
fn pots_seen(i: &engine::SettleInputs) -> (u128, u128, u128) {
    let e = 2 * i.n as u128 + 16;
    let (jst, mst) = (jackpot_start(i.start_jackpot) as u128, mini_start(i.start_jackpot) as u128);
    let (init_j, init_m) = (jackpot_start(i.init_jackpot) as u128, mini_start(i.init_jackpot) as u128);
    let j = i.jackpot_pool as u128 + e * share(S_JACK) as u128 + (e * share(S_JACK) as u128 * (i.seed_pool as u128 + e * share(S_SEED) as u128 + init_j)).div_ceil(jst);
    let m = i.mini_pool as u128 + e * share(S_MINI) as u128 + (e * share(S_MINI) as u128 * init_m).div_ceil(mst);
    let s = i.super_pool as u128 + e * share(S_SUPER) as u128 + (e * share(S_JACK) as u128 * JP_SUPER_BPS as u128 * JP_PAY_CAP_BPS as u128).div_ceil(BPS as u128 * BPS as u128);
    (j, m, s)
}
/// Worth of the pots for the round, points per executed spin (0 for a late settlement: it plays no pots)
fn rich_score(i: &engine::SettleInputs) -> f64 {
    if i.late {
        return 0.0;
    }
    let (cj, cm, cs) = pot_coefficients();
    let (j, m, s) = pots_seen(i);
    let (jst, mst, sst) = (jackpot_start(i.start_jackpot), mini_start(i.start_jackpot), super_start(i.start_super));
    let k = |pot: u128, start: u64, cap_bps: u64| pot.min(pay_cap(start, cap_bps) as u128) as f64 / start as f64;
    (cj as f64 * k(j, jst, JP_PAY_CAP_BPS) + cm as f64 * k(m, mst, MINI_PAY_CAP_BPS) + cs as f64 * k(s, sst, SUPER_PAY_CAP_BPS)) / BPS2
}
fn rich_withheld(i: &engine::SettleInputs) -> bool {
    rich_score(i) > RICH_POT_LIMIT as f64
}

/// The derivation, from the program's constants only. If a share, a winner's share, a pay cap or a prize table changes, the
/// numbers asserted here change and the test fails: the rule of scripts/referrals.ts must then be derived again.
#[test]
fn invite_rule_keeps_every_rewarded_round_below_its_stake() {
    let (cj, cm, cs) = pot_coefficients();
    assert_eq!((cj, cm, cs), (6_688_000_000, 2_933_480_000, 26_370_000_000), "66.88, 29.3348 and 263.7 points per starting amount in the pot");
    // the same numbers the re-verification gives (66.88 / 29.33 / 263.7), and the coefficients the rule had before (83.6 / 44)
    assert_eq!((cj * BPS as u128 / (BPS - JP_KEEP_BPS) as u128, cm * 3 / 2 / 1_000_000), (8_360_000_000, 4_400));
    let table = mode_ev_ppm_tokens(0); // points × 10^6: 7,750.062, the largest of all games (mode_ev_ok)
    assert_eq!(table, 7_750_062_000);
    for m in 1..MODES {
        assert!(mode_ev_ppm_tokens(m) <= table);
    }
    let head = (BPS - W_AGAIN) as u128 * 1_000_000; // 8,840 points: (1 − P(repeat)) of a bet
    let reward = (S_DEV * INVITE_SHARE_BPS) as u128 * 1_000_000 / BPS as u128; // 309 points per executed spin
    let limit = RICH_POT_LIMIT as u128 * 1_000_000;
    assert_eq!(reward, 309_000_000);
    // worst case of a REWARDED round: (table + limit + reward) ÷ 8,840
    let worst = (table + limit + reward) as f64 / head as f64;
    assert!(table + limit + reward < head);
    assert!((worst - 0.996500).abs() < 0.000001, "{worst}");
    // the limit that would be break-even, and the largest share the limit of 750 allows
    let break_even = (head - table - reward) as f64 / 1e6;
    let max_share = (head - table - limit) * BPS as u128 / (S_DEV as u128 * 1_000_000);
    assert!((break_even - 780.938).abs() < 0.001 && max_share == 5_500);
    assert!((table + limit + S_DEV as u128 * 5_500 * 100) < head && (table + limit + S_DEV as u128 * 5_501 * 100) >= head);
    // the rule is not empty and not everything: pots at 1 amount pass, at 2 amounts and at the caps they do not
    let at = |kj: f64, km: f64, ks: f64| (cj as f64 * kj + cm as f64 * km + cs as f64 * ks) / BPS2;
    assert!(at(1.0, 1.0, 1.0) < 750.0 && at(2.0, 2.0, 2.0) < 750.0 && at(2.0, 2.0, 2.25) > 750.0 && (at(3.0, 3.0, 2.5) - 947.8944).abs() < 1e-9);
    // without the rule (every pot at its pay cap) the same player would get 101.89%
    assert!(((table as f64 / 1e6 + at(3.0, 3.0, 2.5) + reward as f64 / 1e6) / 8_840.0 - 1.01889).abs() < 0.00001);
    println!("invite rule: pots worth 66.88·kj + 29.3348·km + 263.7·ks points per executed spin; no reward above {RICH_POT_LIMIT}");
    println!("  pots at 1 amount: {:.1} · at 2: {:.1} · at the caps: {:.2}; with jackpot and mini at 1 amount the reward stops from a super pot of {:.2} targets, at 1.25 amounts from {:.2}, at their caps from {:.2}", at(1.0, 1.0, 1.0), at(2.0, 2.0, 2.0), at(3.0, 3.0, 2.5), (750.0 - at(1.0, 1.0, 0.0)) / 263.7, (750.0 - at(1.25, 1.25, 0.0)) / 263.7, (750.0 - at(3.0, 3.0, 0.0)) / 263.7);
    println!("  worst case of a rewarded round, reward included: ({:.3} + {RICH_POT_LIMIT} + {:.0}) ÷ 8,840 = {:.4}% of the stake; break-even limit {:.1}; largest share with this limit {max_share} bps", table as f64 / 1e6, reward as f64 / 1e6, worst * 100.0, break_even);

    // The same with the program's own chances and payouts, on random states: whenever the rule rewards a round, the exact
    // value of an executed spin at the pots the round can see, plus the reward, is below the bound (and so below the stake).
    let mut sm = Sm(0x1217);
    let reward_lamports = (share(S_DEV) * INVITE_SHARE_BPS / BPS) as u128; // 309,000 per executed spin
    let (mut rewarded, mut withheld, mut closest) = (0u32, 0u32, 0f64);
    for _ in 0..400_000 {
        let j0 = [JP_MIN_POT, SOL, 5 * SOL, 37 * SOL, START_MAX][sm.below(5) as usize];
        let s0 = [SUPER_MIN_POT, 20 * SOL, 80 * SOL, 320 * SOL, START_MAX][sm.below(5) as usize];
        let mul = |sm: &mut Sm, base: u64| (base as f64 * [0.0, 0.2, 0.25, 0.9, 1.0, 1.3, 1.39, 1.5, 2.0, 2.5, 3.0, 10.0][sm.below(12) as usize] * (0.9 + 0.2 * sm.unit())) as u64;
        let mode = sm.below(MODES as u64) as u8;
        let n = [10u16, 10, 25, 100, 1000][sm.below(5) as usize].min(batch_limit_mode(mode, 1));
        let i = engine::SettleInputs {
            mode,
            n,
            free: false,
            late: sm.below(50) == 0,
            prize_cap: u64::MAX,
            start_jackpot: j0,
            start_super: s0,
            prize_pool: 1_000_000 * SOL,
            jackpot_pool: mul(&mut sm, j0),
            mini_pool: mul(&mut sm, mini_start(j0)),
            super_pool: mul(&mut sm, s0),
            seed_pool: sm.below(j0 / 2 + 1),
            seed_reserve: [0, 30 * SOL][sm.below(2) as usize],
            init_jackpot: [j0, j0 * 4, j0 / 2][sm.below(3) as usize].clamp(JP_MIN_POT, START_MAX),
            init_super: s0,
        };
        if rich_withheld(&i) {
            withheld += 1;
            continue;
        }
        rewarded += 1;
        let (j, m, s) = pots_seen(&i);
        let o = if i.late { OddsBase::new_mode(j0, s0, mode).at_super(0, 0, 0, true) } else { OddsBase::new_mode(j0, s0, mode).at_super(u64::MAX / 4, u64::MAX / 4, u64::MAX / 4, true) };
        // the wins as the engine computes them: the kept share is rounded down, so the win can be a lamport above the formula
        let (jt, mt) = (j.min(pay_cap(jackpot_start(j0), JP_PAY_CAP_BPS) as u128) as u64, m.min(pay_cap(mini_start(j0), MINI_PAY_CAP_BPS) as u128) as u64);
        let mul_bps = |v: u64, b: u64| (v as u128 * b as u128 / BPS as u128) as u64; // as in engine.rs
        let jw = mul_bps(jt - mul_bps(jt, JP_KEEP_BPS), JP_WINNER_BPS);
        let mw = mt - mul_bps(mt, MINI_KEEP_BPS);
        let sw = (s.min(pay_cap(super_start(s0), SUPER_PAY_CAP_BPS) as u128) * SUPER_WINNER_BPS as u128 / BPS as u128) as u64;
        let ev = spin_ev_q32(&o, mode, jw, mw, sw) + (reward_lamports << 32);
        let den = Q32 - o.again as u128;
        let v = ev as f64 / den as f64 / COST as f64;
        closest = closest.max(v);
        assert!(v <= worst + 1e-9 && ev < COST as u128 * den, "a rewarded round worth {v} of its stake: {i:?}");
    }
    println!("  400,000 random states with the program's chances: {rewarded} rewarded, {withheld} withheld; the best rewarded one is worth {:.4}% of the stake with the reward", closest * 100.0);
    assert!(rewarded > 50_000 && withheld > 50_000 && closest > 0.99 && closest <= worst + 1e-9);
}

/// The vectors both implementations of the rule are checked on (this one and `richPot` of scripts/referrals.ts).
/// FD_RICH_VECTORS_WRITE=1 rewrites the file; otherwise the file must already hold exactly what is produced here.
#[test]
fn invite_rule_vectors() {
    let mut sm = Sm(0xF1C5);
    let mut out = String::from("{\"limit\":750,\"vectors\":[\n");
    let (mut yes, mut no) = (0, 0);
    let mut k = 0;
    while k < 400 {
        let j0 = [0, JP_MIN_POT, SOL, 5 * SOL, START_MAX][sm.below(5) as usize];
        let s0 = [0, SUPER_MIN_POT, 20 * SOL, 320 * SOL, START_MAX][sm.below(5) as usize];
        let mul = |sm: &mut Sm, base: u64| (base as f64 * [0.0, 0.25, 1.0, 1.39, 2.0, 3.0, 10.0][sm.below(7) as usize] * (0.8 + 0.4 * sm.unit())) as u64;
        let i = engine::SettleInputs {
            mode: sm.below(MODES as u64) as u8,
            n: [1u16, 10, 100, 1000][sm.below(4) as usize],
            free: false,
            late: sm.below(20) == 0,
            prize_cap: sm.below(100 * SOL),
            start_jackpot: j0,
            start_super: s0,
            prize_pool: sm.below(1000 * SOL),
            jackpot_pool: mul(&mut sm, jackpot_start(j0)),
            mini_pool: mul(&mut sm, mini_start(j0)),
            super_pool: mul(&mut sm, super_start(s0)),
            seed_pool: sm.below(jackpot_start(j0)),
            seed_reserve: sm.below(50 * SOL),
            init_jackpot: [j0, j0.saturating_mul(4).min(START_MAX), j0 / 3][sm.below(3) as usize],
            init_super: s0,
        };
        let score = rich_score(&i);
        if (score - RICH_POT_LIMIT as f64).abs() < 1e-3 {
            continue; // the float version must not decide a tie: the exact one (TypeScript, integers) has its own edge test
        }
        let w = rich_withheld(&i);
        if w {
            yes += 1
        } else {
            no += 1
        }
        out += &format!(
            "{{\"n\":{},\"late\":{},\"startJackpot\":\"{}\",\"startSuper\":\"{}\",\"jackpotPool\":\"{}\",\"miniPool\":\"{}\",\"superPool\":\"{}\",\"seedPool\":\"{}\",\"seedReserve\":\"{}\",\"initJackpot\":\"{}\",\"initSuper\":\"{}\",\"score\":{:.4},\"withheld\":{}}}{}\n",
            i.n, i.late, i.start_jackpot, i.start_super, i.jackpot_pool, i.mini_pool, i.super_pool, i.seed_pool, i.seed_reserve, i.init_jackpot, i.init_super, score, w, if k == 399 { "" } else { "," }
        );
        k += 1;
    }
    out += "]}\n";
    assert!(yes > 80 && no > 80, "{yes} withheld, {no} rewarded");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/local/fixtures/rich-pot-vectors.json");
    if std::env::var("FD_RICH_VECTORS_WRITE").is_ok() {
        std::fs::write(&path, &out).unwrap();
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), out, "tests/local/fixtures/rich-pot-vectors.json is out of date: run with FD_RICH_VECTORS_WRITE=1 and re-run the referral tests");
    println!("rich-pot vectors: 400 ({yes} withheld, {no} rewarded) equal to tests/local/fixtures/rich-pot-vectors.json");
}

/// Measured with the engine at the EDGE of the rule (the richest state that is still rewarded): jackpot and mini at their pay
/// caps, the super pot just under the limit, the smallest amounts (so pot wins are frequent and the sample is tight), rounds
/// of 10 bets. The player gets his winnings plus the whole reward. Must agree with the exact expectation and stay below 100%.
#[test]
fn invite_rule_engine_at_the_edge_of_the_rule() {
    let bets: u64 = std::env::var("FD_ECON_EDGE_BETS").ok().and_then(|s| s.parse().ok()).unwrap_or(100_000_000);
    let (mut c, _) = bank(1_000_000 * SOL, JP_MIN_POT, SUPER_MIN_POT, 0);
    let mut p = Player::default();
    engine::init_player(&mut p, key(1), 255, 1, T0);
    let mut sm = Sm(77_001);
    // the largest super pot for which a round of 10 bets is still rewarded with jackpot and mini at ×3 (found by bisection on the rule)
    let state = |c: &mut Config, sup: u64| {
        c.jackpot_pool = 3 * JP_MIN_POT;
        c.mini_pool = 3 * MINI_MIN_POT;
        c.super_pool = sup;
        c.seed_pool = 0;
        c.prize_pool = 1_000_000 * SOL;
    };
    let (mut lo, mut hi) = (0u64, 3 * SUPER_MIN_POT);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        state(&mut c, mid);
        p.balance = 10 * COST;
        engine::commit(&mut c, &mut p, T0 + 1, 1, 10, false, key(9)).unwrap();
        let w = rich_withheld(&engine::round_inputs(&c, &p, T0 + 2, false));
        engine::refund(&mut c, &mut p, T0 + 2).unwrap();
        p.balance = 0;
        if w {
            hi = mid
        } else {
            lo = mid
        }
    }
    let edge = lo;
    let (mut paid, mut got, mut exec) = (0u128, 0u128, 0u128);
    for k in 0..bets / 10 {
        state(&mut c, edge);
        p.balance = 10 * COST;
        p.jp_locked = 0;
        p.jp_per_week = 0;
        engine::commit(&mut c, &mut p, T0 + 1, k + 2, 10, false, key(9)).unwrap();
        assert!(!rich_withheld(&engine::round_inputs(&c, &p, T0 + 2, false)));
        let out = engine::settle(&mut c, &mut p, Some(sm.seed()), T0 + 2).unwrap();
        paid += 10 * COST as u128;
        got += p.balance as u128 + p.jp_locked as u128;
        exec += out.spins as u128;
        p.balance = 0;
    }
    let reward = exec * (share(S_DEV) * INVITE_SHARE_BPS / BPS) as u128;
    let o = OddsBase::new_mode(JP_MIN_POT, SUPER_MIN_POT, 0).at_super(u64::MAX / 4, u64::MAX / 4, u64::MAX / 4, true);
    let exact = (spin_ev_q32(&o, 0, 3 * JP_MIN_POT / 100 * 80 / 100 * 95, 3 * MINI_MIN_POT - 3 * MINI_MIN_POT * MINI_KEEP_BPS / BPS, (edge as u128 * 9 / 10) as u64) + ((share(S_DEV) as u128 * INVITE_SHARE_BPS as u128 / BPS as u128) << 32)) as f64 / (Q32 - o.again as u128) as f64 / COST as f64;
    let (plain, with) = (got as f64 / paid as f64, (got + reward) as f64 / paid as f64);
    println!(
        "{bets} bets placed only at the edge of the rule (jackpot and mini at ×3, super pot {:.4} of its amount, rounds of 10): returned {:.3}% + reward {:.3}% = {:.3}% (exact expectation {:.3}%, bound 99.650%)",
        edge as f64 / SUPER_MIN_POT as f64, plain * 100.0, reward as f64 / paid as f64 * 100.0, with * 100.0, exact * 100.0
    );
    // with the exact coefficients the rewarded round at the edge of the rule is worth the rule's own bound of 99.650%, not more
    assert!(exact <= 0.996501 && exact > 0.9960, "{exact}");
    assert!(with < 1.0);
    assert!((with - exact).abs() < 0.004, "measured {with}, exact {exact}");
}

/// How much volume the rule withholds in ordinary play, and what a partner's players get. 40 players, every game, rounds of
/// 10…1000 bets, jackpot amount 5 SOL, the super target raised in the allowed ×4 steps to `target`; the first tenth of the run
/// is not counted. `donate`: the owner keeps the reserve filled (the jackpot and the mini then restart at full size).
fn invite_rule_population(target_sol: u64, donate: bool, stakes: u64, seed: u64) -> (f64, f64, f64, f64, [(f64, f64, f64); 6]) {
    let (mut c, _) = bank(200 * SOL, 5 * SOL, 20 * SOL, 30 * SOL);
    let mut sm = Sm(seed);
    let np = 40usize;
    let mut ps: Vec<Player> = (0..np as u64)
        .map(|i| {
            let mut p = Player::default();
            let n = engine::assign_number(&mut c).unwrap();
            engine::init_player(&mut p, key(i), 255, n, T0);
            p
        })
        .collect();
    let mut now = T0 + 10;
    let warm = stakes / 10;
    let (mut paid, mut rounds) = (0u64, 0u64);
    let mut donated = 0u128; // what the owner put into the reserve to keep it at 30 SOL: all of it went into jackpot and mini restarts
    // by state of the rule: [rewarded, withheld] → (stake, winnings, executed spins)
    let mut tot = [(0u128, 0u128, 0u128); 2];
    // by the round's score: < 300, 300–450, 450–600, 600–750, 750–900, > 900
    let mut bins = [(0u128, 0u128, 0u128); 6];
    while paid < stakes {
        rounds += 1;
        now += 300;
        if rounds % 300 == 0 {
            let cur = engine::super_start_of(&c);
            if cur < target_sol * SOL && c.starts_apply_ts == 0 {
                let next = (target_sol * SOL).min(engine::starts_raise_cap(c.super_pool, cur)).max(c.super_pool / 2);
                let _ = engine::set_starts(&mut c, 5 * SOL, next, now);
            }
            if donate && c.seed_reserve < 30 * SOL {
                let add = 30 * SOL - c.seed_reserve;
                engine::donate(&mut c, add).unwrap();
                donated += add as u128;
            }
            c.dev_pool = 0;
        }
        let i = sm.below(np as u64) as usize;
        let p = &mut ps[i];
        let mode = if sm.below(3) == 0 { 0 } else { sm.below(MODES as u64) as u8 };
        let n = if mode == 0 { [10u16, 10, 20, 50, 100, 100, 250, 500, 1000][sm.below(9) as usize] } else { [10u16, 10, 20, 25, 50, 100][sm.below(6) as usize] };
        let stake = n as u64 * COST;
        p.balance = stake;
        p.jp_locked = 0;
        p.jp_per_week = 0;
        engine::commit_mode(&mut c, p, now, rounds, n, mode, false, key(9)).unwrap();
        let inp = engine::round_inputs(&c, p, now + 2, false);
        let score = rich_score(&inp);
        let out = engine::settle(&mut c, p, Some(sm.seed()), now + 2).unwrap();
        paid += n as u64;
        if paid < warm {
            continue;
        }
        let won = p.balance as u128 + p.jp_locked as u128;
        let w = (score > RICH_POT_LIMIT as f64) as usize;
        for t in [&mut tot[w], &mut bins[[300.0, 450.0, 600.0, 750.0, 900.0].iter().filter(|x| score > **x).count()]] {
            t.0 += stake as u128;
            t.1 += won;
            t.2 += out.spins as u128;
        }
    }
    if donate {
        // (docs/UPGRADE-2.md 12.2) less than the 44 + 70 points of "every restart at full size": what a capped win leaves in
        // the pot, and the seed pool, make the top-ups smaller
        println!("        reserve kept filled: the owner's donations were {:.3}% of the paid bets ({:.0} SOL on {:.0} SOL)", donated as f64 / (paid as f64 * COST as f64) * 100.0, donated as f64 / 1e9, paid as f64 / 100.0);
    }
    let rw = (share(S_DEV) * INVITE_SHARE_BPS / BPS) as f64;
    let ret = |t: (u128, u128, u128), reward: bool| if t.0 == 0 { 0.0 } else { (t.1 as f64 + if reward { t.2 as f64 * rw } else { 0.0 }) / t.0 as f64 * 100.0 };
    let all = (tot[0].0 + tot[1].0) as f64;
    let mut b = [(0.0, 0.0, 0.0); 6];
    for k in 0..6 {
        b[k] = (bins[k].0 as f64 / all * 100.0, ret(bins[k], false), ret(bins[k], k < 4));
    }
    // share of volume withheld; return of the rewarded rounds with the reward; of the withheld rounds (no reward); of everything a partner's player gets
    (tot[1].0 as f64 / all * 100.0, ret(tot[0], true), ret(tot[1], false), (tot[0].1 as f64 + tot[0].2 as f64 * rw + tot[1].1 as f64) / all * 100.0, b)
}

#[test]
fn invite_rule_share_of_volume_withheld() {
    let stakes: u64 = std::env::var("FD_ECON_RULE_STAKES").ok().and_then(|s| s.parse().ok()).unwrap_or(60_000_000);
    println!("share of the stakes that get no invite reward by the rich-pot rule ({stakes} bets = {} SOL per line; the first tenth not counted):", stakes / 100);
    println!("  super target | reserve | withheld | rewarded rounds return with reward | withheld rounds return (no reward) | all rounds of a partner's player");
    let mut shares = Vec::new();
    for (target, donate) in [(80u64, false), (320, false), (320, true), (1_000, false)] {
        let (w, r_in, r_out, r_all, bins) = invite_rule_population(target, donate, stakes, 0xD1CE + target);
        println!("  {target:>5} SOL    | {:<7} | {w:>5.1}%   | {r_in:>6.2}%                             | {r_out:>6.2}%                           | {r_all:>6.2}%", if donate { "filled" } else { "empty" });
        // (the returns of the single bands are not printed: a band holds few super jackpot cycles and its sample is too noisy)
        println!("        stakes by the round's score (<300, 300–450, 450–600, 600–750 | 750–900, >900): {}", bins.iter().map(|b| format!("{:.1}%", b.0)).collect::<Vec<_>>().join(" · "));
        shares.push(w);
    }
    // the rule must leave most of the volume rewarded and must not be idle
    assert!(shares.iter().all(|w| *w > 2.0 && *w < 45.0), "{shares:?}");
}
