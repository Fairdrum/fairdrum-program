//! Tests of upgrade No. 2 (changes A–D on top of the first prepared version). Pure logic, no Solana.
//! Run: `cargo test -p fairdrum --release --offline --test upgrade2_tests -- --nocapture`
//!
//!  A — the RoundInputs event: its fields and the round number are enough to repeat the settlement;
//!  B — bounds on the starting amounts (audit No. 9, G1 and G2) and the fact that ordinary administrator actions remain possible;
//!  C — withdrawal in regular SOL: the accounting is the same as for the previous withdrawal; the path of the funds is checked on a model (NOT on a network — see the test);
//!  D — see engine_tests.rs::set_root_equals_live_set_house_root.

use anchor_lang::prelude::Pubkey;
use fairdrum::consts::*;
use fairdrum::engine;
use fairdrum::errors::FdError;
use fairdrum::math::*;
use fairdrum::state::*;

const T0: i64 = 1_800_000_000;
const SOL: u64 = 1_000_000 * ONE;
const DAY: i64 = 86_400;

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
fn key(i: u8) -> Pubkey {
    Pubkey::new_from_array([i; 32])
}
fn cfg_with(prize: u64, jackpot: u64, superj: u64, reserve: u64) -> Config {
    let mut c = Config::default();
    c.start_ts = T0;
    c.init_prize = prize;
    c.init_seed_reserve = reserve;
    c.init_jackpot = jackpot;
    c.init_super = superj;
    engine::fund(&mut c, prize + jackpot + superj + reserve).unwrap();
    c
}
fn player(c: &mut Config, i: u8, now: i64) -> Player {
    let n = engine::assign_number(c).unwrap();
    let mut p = Player::default();
    engine::init_player(&mut p, key(i), 255, n, now);
    p
}
fn is_err(r: anchor_lang::Result<()>, e: FdError) -> bool {
    match r {
        Err(x) => x == e.into(),
        Ok(()) => false,
    }
}

// =====================================================================================================================
// A. RoundInputs
// =====================================================================================================================

/// What a third party sees: the RoundInputs event and the round number. We repeat the settlement on EMPTY accounts filled only
/// from the event and compare with the real settlement: the round totals, the six pools after settlement, the credits to the player.
fn replay_from_event(i: &engine::SettleInputs, seed: [u8; 32], now: i64) -> (engine::SettleOut, [u64; 7], [u64; 3]) {
    let mut c = Config::default();
    c.funded = true;
    c.start_ts = T0;
    c.epoch_start = now; // the statistics week does not affect the outcome
    c.den = DEN_MIN;
    c.prize_pool = i.prize_pool;
    c.jackpot_pool = i.jackpot_pool;
    c.mini_pool = i.mini_pool;
    c.super_pool = i.super_pool;
    c.seed_pool = i.seed_pool;
    c.seed_reserve = i.seed_reserve;
    c.init_jackpot = i.init_jackpot;
    c.init_super = i.init_super;
    let mut p = Player::default();
    p.pending = true;
    p.p_n = pack_pending(i.n, i.mode);
    p.p_free = i.free;
    p.p_stake = i.n as u64 * COST;
    p.p_prize_cap = i.prize_cap;
    p.p_start_jackpot = i.start_jackpot;
    p.p_start_super = i.start_super;
    let out = if i.late { engine::settle_late(&mut c, &mut p, seed, now).unwrap() } else { engine::settle(&mut c, &mut p, Some(seed), now).unwrap() };
    (out, [c.prize_pool, c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_pool, c.seed_reserve, c.promo_pool], [p.balance, p.jp_locked, p.jp_per_week])
}
fn same_out(a: &engine::SettleOut, b: &engine::SettleOut) -> bool {
    (a.spins, a.extras, a.gold, a.prizes, a.best_prize, a.mini_hits, a.mini_won) == (b.spins, b.extras, b.gold, b.prizes, b.best_prize, b.mini_hits, b.mini_won)
        && (a.jp_hits, a.jp_pool_total, a.jp_to_player_now, a.super_hits, a.super_total, a.forfeited, a.refunded) == (b.jp_hits, b.jp_pool_total, b.jp_to_player_now, b.super_hits, b.super_total, b.forfeited, b.refunded)
}

#[test]
fn a_round_inputs_and_round_value_reproduce_the_settlement() {
    // A live game: 12 players, all modes, a small bank (prizes are cut by the cap), the smallest starting amounts
    // (pots are won often), the administrator changes the starting amounts and tops up the bank from time to time.
    let (mut all_due, mut all_capped) = (0u64, 0u64);
    for (prize, jp, sp, reserve, rounds) in [(SOL / 2, JP_MIN_POT, SUPER_MIN_POT, SOL, 30_000u64), (60 * SOL, 5 * SOL, 20 * SOL, 0, 12_000)] {
        let mut c = cfg_with(prize, jp, sp, reserve);
        let mut r = Sm(0xA11CE ^ prize);
        let mut now = T0 + 10;
        let mut ps: Vec<Player> = (0..12).map(|i| player(&mut c, i + 1, now)).collect();
        let (mut done, mut late_n, mut capped, mut pots, mut deferred_due, mut cap_differs) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
        for step in 0..rounds {
            now += 1 + r.below(4_000) as i64;
            let i = r.below(ps.len() as u64) as usize;
            if !ps[i].pending {
                engine::deposit(&mut ps[i], 12 * SOL, now).unwrap();
                let mode = r.below(MODES as u64) as u8;
                let n = if mode == 0 { [1u16, 10, 100, 1000][r.below(4) as usize] } else { [1u16, 7, 10, 100][r.below(4) as usize] };
                engine::commit_mode(&mut c, &mut ps[i], now, step, n, mode, false, key(9)).unwrap();
            }
            // the administrator between bet and settlement: change of starting amounts (after 24 hours), move of the reserve into the bank, donation
            match r.below(40) {
                0 => {
                    let _ = engine::set_starts(&mut c, JP_MIN_POT + r.below(3 * SOL), SUPER_MIN_POT + r.below(6 * SOL), now);
                }
                1 if c.seed_reserve > 0 => {
                    // accepted exactly while the bank ends at or below BANK_KEEP (the banks of this test are far below it)
                    let amt = 1 + r.below(c.seed_reserve);
                    let fits = c.prize_pool as u128 + amt as u128 <= BANK_KEEP as u128;
                    assert_eq!(engine::rescale(&mut c, amt, 0, 0).is_ok(), fits);
                }
                2 => engine::donate(&mut c, 1 + r.below(2 * SOL)).unwrap(),
                3 => now += DAY, // a pending change comes due
                _ => {}
            }
            if r.below(3) != 0 {
                let late = r.below(6) == 0;
                let seed = r.seed();
                // before the settlement — exactly what the event publishes
                let inp = engine::round_inputs(&c, &ps[i], now, late);
                let due = c.starts_apply_ts != 0 && now >= c.starts_apply_ts;
                deferred_due += due as u64;
                let promo0 = c.promo_pool;
                let (b0, l0, w0) = (ps[i].balance, ps[i].jp_locked, ps[i].jp_per_week);
                let mut vested = ps[i].clone();
                let released = engine::release_jackpot(&mut vested, now); // the settlement first pays out the vested part of previous jackpots
                let out = if late { engine::settle_late(&mut c, &mut ps[i], seed, now).unwrap() } else { engine::settle(&mut c, &mut ps[i], Some(seed), now).unwrap() };
                // the starting amounts in the event are those that were in effect in the settlement
                assert_eq!((inp.init_jackpot, inp.init_super), (c.init_jackpot, c.init_super), "step {step}");
                let (out2, pools2, gain2) = replay_from_event(&inp, seed, now);
                assert!(same_out(&out, &out2), "step {step}: the round total per the event differs\n{out:?}\n{out2:?}");
                assert_eq!([c.prize_pool, c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_pool, c.seed_reserve, c.promo_pool - promo0], pools2, "step {step}: pools after settlement");
                assert_eq!(ps[i].balance - b0 - released, gain2[0], "step {step}: credited to the balance");
                assert_eq!(ps[i].jp_locked + released - l0, gain2[1], "step {step}: frozen");
                if l0 == 0 || vested.jp_locked > 0 {
                    assert_eq!(ps[i].jp_per_week - if vested.jp_locked == 0 { 0 } else { w0 }, gain2[2], "step {step}: payout per week");
                }
                done += 1;
                late_n += late as u64;
                pots += (out.mini_hits + out.jp_hits + out.super_hits) as u64;
                capped += (out.gold > 0 && inp.prize_cap.min(inp.prize_pool / PRIZE_BANK_DIV) < 10 * SOL) as u64;
                cap_differs += (inp.prize_cap != inp.prize_pool / PRIZE_BANK_DIV) as u64;
            }
        }
        println!("bank {} SOL: settlements {done} (late {late_n}), with the prize cap in effect {capped}, cap at bet time ≠ a quarter of the bank at settlement {cap_differs}, pots won {pots}, settlements with a due change of starting amounts {deferred_due}", prize / SOL);
        assert!(done > 5_000 && late_n > 500 && pots > 20 && cap_differs > 1_000);
        all_due += deferred_due;
        all_capped += capped;
    }
    assert!(all_due > 3 && all_capped > 100, "the scenarios must include both a due change of starting amounts and cut prizes");
}

#[test]
fn a_round_inputs_reads_deferred_starts_exactly_like_the_settlement() {
    let mut c = cfg_with(50 * SOL, 5 * SOL, 20 * SOL, 100 * SOL);
    let mut p = player(&mut c, 1, T0);
    engine::deposit(&mut p, SOL, T0).unwrap();
    engine::commit_mode(&mut c, &mut p, T0 + 1, 5, 10, 3, false, key(9)).unwrap();
    engine::set_starts(&mut c, 6 * SOL, 21 * SOL, T0 + 2).unwrap();
    let at = c.starts_apply_ts;
    // before the deadline — the previous amounts, at the deadline and after — the new ones; the bet's amounts (p_start_*) never change
    for (now, want) in [(at - 1, (5 * SOL, 20 * SOL)), (at, (6 * SOL, 21 * SOL)), (at + 1, (6 * SOL, 21 * SOL))] {
        let i = engine::round_inputs(&c, &p, now, false);
        assert_eq!((i.init_jackpot, i.init_super), want);
        assert_eq!((i.start_jackpot, i.start_super, i.mode, i.n, i.free, i.late), (5 * SOL, 20 * SOL, 3, 10, false, false));
        assert_eq!(i.prize_cap, 50 * SOL / PRIZE_BANK_DIV);
        let (mut c2, mut p2) = (c.clone(), p.clone());
        engine::settle(&mut c2, &mut p2, Some([3; 32]), now).unwrap();
        assert_eq!((c2.init_jackpot, c2.init_super), want, "the settlement applied (or did not apply) the change the same way");
    }
    // reading the inputs changes nothing (the function takes the accounts read-only) — we compare the bytes just in case
    let bytes = |c: &Config, p: &Player| {
        let mut v = Vec::new();
        anchor_lang::AccountSerialize::try_serialize(c, &mut v).unwrap();
        anchor_lang::AccountSerialize::try_serialize(p, &mut v).unwrap();
        v
    };
    let b0 = bytes(&c, &p);
    let _ = engine::round_inputs(&c, &p, at + 5, true);
    assert_eq!(b0, bytes(&c, &p));
}

/// Event size: 8 bytes of discriminator + 133 bytes of fields. In the log it is a single "Program data: …" line of 188 characters.
#[test]
fn a_round_inputs_event_size() {
    let ev = RoundInputs {
        owner: key(1),
        commit_slot: 1,
        mode: 15,
        n: 1000,
        free: false,
        late: true,
        prize_cap: u64::MAX,
        start_jackpot: 1,
        start_super: 2,
        prize_pool: 3,
        jackpot_pool: 4,
        mini_pool: 5,
        super_pool: 6,
        seed_pool: 7,
        seed_reserve: 8,
        init_jackpot: 9,
        init_super: 10,
    };
    let data = anchor_lang::Event::data(&ev);
    assert_eq!(data.len(), 8 + 32 + 8 + 1 + 2 + 1 + 1 + 11 * 8);
    assert_eq!(data.len(), 141);
    assert_eq!((data.len() + 2) / 3 * 4, 188);
    // field order in the bytes — as in the IDL: owner, commit_slot, mode, n, free, late, then eleven amounts
    assert_eq!(&data[8..40], &[1u8; 32]);
    assert_eq!(&data[40..48], &1u64.to_le_bytes());
    assert_eq!(&data[48..53], &[15, 0xE8, 0x03, 0, 1]);
    assert_eq!(&data[53..61], &u64::MAX.to_le_bytes());
    assert_eq!(&data[133..141], &10u64.to_le_bytes());
}

// =====================================================================================================================
// B. Bounds on the starting amounts (audit No. 9: G1, G2)
// =====================================================================================================================

/// G1, the auditor's scenario word for word: starting jackpot 5 SOL, the reserve is empty, the jackpot has just been won (the pot holds
/// 0.25 SOL), the mini has accumulated 3 SOL. The live program accepted a decrease to 0.2 SOL — the mini chance grew 25-fold.
#[test]
fn b_g1_lowering_cannot_make_the_mini_pot_easy() {
    let mut c = cfg_with(100 * SOL, 5 * SOL, 2 * SOL, 0);
    c.jackpot_pool = SOL / 4;
    c.mini_pool = 3 * SOL;
    let s0 = c.init_super;
    assert!(is_err(engine::set_starts(&mut c, JP_MIN_POT, s0, T0 + 20), FdError::BadParams), "a decrease under an accumulated mini is rejected");
    assert_eq!(c.starts_apply_ts, 0, "the rejected command wrote nothing");
    // bound: the mini starting amount (a tenth of the jackpot) is not below half of the mini pot → the jackpot is not below 15 SOL;
    // any decrease from 5 SOL is forbidden for now, until the mini is won
    for js in [JP_MIN_POT, SOL, 4 * SOL, 5 * SOL - 1] {
        assert!(engine::set_starts(&mut c, js, s0, T0 + 20).is_err(), "{js}");
    }
    // the mini was won — the mini pot is at its starting amount again (0.5 SOL): a decrease is allowed, but not below 2.5 SOL (mini 0.25 ≥ 0.5 / 2)
    c.mini_pool = SOL / 2;
    assert!(engine::set_starts(&mut c, 5 * SOL / 2 - 1, s0, T0 + 20).is_err());
    engine::set_starts(&mut c, 5 * SOL / 2, s0, T0 + 20).unwrap();
}

/// Proviso to G1: the mini check applies only on a DECREASE. Otherwise the administrator could not change the super jackpot amount
/// alone (or raise the jackpot) while the mini holds more than two of its starting amounts — and that is the usual state.
#[test]
fn b_g1_does_not_block_keeping_or_raising_the_jackpot_start() {
    let mut c = cfg_with(100 * SOL, 5 * SOL, 20 * SOL, 100 * SOL);
    c.mini_pool = 4 * SOL; // eight mini starting amounts
    engine::set_starts(&mut c, 5 * SOL, 25 * SOL, T0 + 5).expect("the jackpot amount is the same, only the super changes");
    engine::set_starts(&mut c, 6 * SOL, 20 * SOL, T0 + 6).expect("raising the jackpot only reduces the mini chance");
    assert!(engine::set_starts(&mut c, 5 * SOL - 1, 20 * SOL, T0 + 7).is_err(), "but a decrease is not allowed");
}

/// G2, the auditor's scenario: both starting amounts set to 1000 SOL at once. The live program accepted it — the pots closed.
#[test]
fn b_g2_raise_is_limited_to_four_times() {
    let mut c = cfg_with(100 * SOL, 5 * SOL, 20 * SOL, 30 * SOL);
    assert!(is_err(engine::set_starts(&mut c, START_MAX, START_MAX, T0 + 5), FdError::BadParams));
    assert!(engine::set_starts(&mut c, 20 * SOL + 1, 20 * SOL, T0 + 5).is_err(), "jackpot: more than fourfold");
    assert!(engine::set_starts(&mut c, 5 * SOL, 80 * SOL + 1, T0 + 5).is_err(), "super: more than fourfold");
    engine::set_starts(&mut c, 20 * SOL, 80 * SOL, T0 + 5).expect("exactly fourfold is allowed");
    // the point of the bound: a pot that holds the starting amount in effect remains winnable after the raise
    engine::apply_starts_if_due(&mut c, T0 + 5 + STARTS_DELAY_SECS);
    assert!(c.jackpot_pool >= open_pot(jackpot_start(c.init_jackpot)) && c.super_pool >= open_pot(super_start(c.init_super)));
    // the mini too (its starting amount is a tenth of the jackpot): 0.5 SOL ≥ a quarter of 2 SOL
    assert!(c.mini_pool >= open_pot(mini_start(c.init_jackpot)));
    // until the change takes effect, the next "step" cannot be set: the count goes from the amount in effect
    let mut d = cfg_with(100 * SOL, 5 * SOL, 20 * SOL, 30 * SOL);
    engine::set_starts(&mut d, 20 * SOL, 80 * SOL, T0 + 5).unwrap();
    assert!(engine::set_starts(&mut d, 80 * SOL, 80 * SOL, T0 + 6).is_err());
    assert!(engine::set_starts(&mut d, 80 * SOL, 80 * SOL, T0 + 5 + STARTS_DELAY_SECS - 1).is_err());
    engine::set_starts(&mut d, 80 * SOL, 320 * SOL, T0 + 5 + STARTS_DELAY_SECS).expect("after 24 hours — the next step");
    // a large pot allows raising to four pots at once: it stays open in the process
    let mut e = cfg_with(100 * SOL, 5 * SOL, 20 * SOL, 0);
    e.jackpot_pool = 30 * SOL;
    engine::set_starts(&mut e, 120 * SOL, 20 * SOL, T0 + 5).unwrap();
    assert!(engine::set_starts(&mut e, 120 * SOL + 1, 20 * SOL, T0 + 5).is_err());
    assert_eq!(engine::starts_raise_cap(u64::MAX / 2, 1), START_MAX, "no overflow, not above START_MAX");
}

/// The owner's ordinary action after the upgrade: raise the jackpot from the smallest 0.2 SOL to 5 SOL, the super from 2 to 20 SOL.
/// Prints the schedule; asserts that this takes three commands and three days until the last one takes effect.
#[test]
fn b_raising_from_minimum_to_five_sol_takes_three_steps() {
    // mainnet state per HANDOVER: bank 0.5 SOL, the smallest starting amounts, the reserve is empty
    let mut c = cfg_with(SOL / 2, 0, 0, 0);
    assert_eq!((c.init_jackpot, c.init_super), (JP_MIN_POT, SUPER_MIN_POT));
    let (goal_j, goal_s) = (5 * SOL, 20 * SOL);
    let mut now = T0 + 100;
    let mut steps = 0;
    assert!(engine::set_starts(&mut c, goal_j, goal_s, now).is_err(), "not possible with one command");
    loop {
        engine::apply_starts_if_due(&mut c, now);
        let (cur_j, cur_s) = (engine::jackpot_start_of(&c), engine::super_start_of(&c));
        if (cur_j, cur_s) == (goal_j, goal_s) {
            break;
        }
        let j = goal_j.min(engine::starts_raise_cap(c.jackpot_pool, cur_j));
        let s = goal_s.min(engine::starts_raise_cap(c.super_pool, cur_s));
        engine::set_starts(&mut c, j, s, now).unwrap();
        steps += 1;
        println!("command {steps} (day {}): jackpot {:.1} → {:.1} SOL, super {:.0} → {:.0} SOL; in effect after 24 hours", (now - T0 - 100) / DAY, cur_j as f64 / SOL as f64, j as f64 / SOL as f64, cur_s as f64 / SOL as f64, s as f64 / SOL as f64);
        now += STARTS_DELAY_SECS;
    }
    assert_eq!(steps, 3, "0,2 → 0,8 → 3,2 → 5 SOL");
    assert_eq!(now - (T0 + 100), 3 * DAY, "the last amount takes effect three days after the first command");
    // and downwards: from 5 SOL back — while the jackpot pot is no more than two new amounts and the mini no more than two new mini amounts
    c.jackpot_pool = SOL;
    c.mini_pool = SOL / 10;
    engine::set_starts(&mut c, SOL / 2, goal_s, now).unwrap();
}

/// The bounds do not interfere with settlements and bets: a rejected change changes nothing, an accepted one — as before, after 24 hours.
#[test]
fn b_bounds_leave_everything_else_as_it_was() {
    let mut c = cfg_with(50 * SOL, 5 * SOL, 20 * SOL, 100 * SOL);
    let now = T0 + 10;
    // everything that passed on the live program and does not violate the new bounds passes and writes the same thing
    engine::set_starts(&mut c, 3 * SOL, 12 * SOL, now).unwrap();
    assert_eq!((c.next_jackpot, c.next_super, c.starts_apply_ts), (3 * SOL, 12 * SOL, now + STARTS_DELAY_SECS));
    assert_eq!((c.init_jackpot, c.init_super), (5 * SOL, 20 * SOL), "before the deadline the previous ones apply");
    // the previous bounds are in place
    assert!(engine::set_starts(&mut c, JP_MIN_POT - 1, 20 * SOL, now).is_err());
    assert!(engine::set_starts(&mut c, 5 * SOL, SUPER_MIN_POT - 1, now).is_err());
    assert!(engine::set_starts(&mut c, 2 * SOL, 20 * SOL, now).is_err(), "below half of the jackpot pot");
    assert!(engine::set_starts(&mut c, 5 * SOL, 9 * SOL, now).is_err(), "below half of the super pot");
}

/// Verification of 18.11.2026, finding 1: the checks are per pot. A super pot above 2 × START_MAX (2,000 SOL) used to make
/// EVERY command fail — the jackpot amount could not be changed either until the super jackpot was won.
#[test]
fn b_a_rich_pot_blocks_only_its_own_amount() {
    // the super pot holds 2,500 SOL at a 1,000 SOL amount: no super amount passes (≥ 1,250 is above START_MAX)
    let mut c = cfg_with(100 * SOL, 5 * SOL, 20 * SOL, 0);
    c.init_super = START_MAX;
    c.super_pool = 2_500 * SOL;
    for s in [SUPER_MIN_POT, 999 * SOL, START_MAX - 1, START_MAX + 1, 1_250 * SOL, u64::MAX] {
        assert!(is_err(engine::set_starts(&mut c, 5 * SOL, s, T0 + 5), FdError::BadParams), "super {s}");
        assert!(is_err(engine::set_starts(&mut c, 6 * SOL, s, T0 + 5), FdError::BadParams), "super {s} together with a valid jackpot change");
    }
    assert_eq!(c.starts_apply_ts, 0, "the rejected commands wrote nothing");
    // ...but the jackpot amount can be changed, with every bound of its own still in force
    engine::set_starts(&mut c, 20 * SOL, START_MAX, T0 + 5).expect("jackpot ×4 while the super amount stays");
    assert_eq!((c.next_jackpot, c.next_super, c.starts_apply_ts), (20 * SOL, START_MAX, T0 + 5 + STARTS_DELAY_SECS));
    assert!(is_err(engine::set_starts(&mut c, 20 * SOL + 1, START_MAX, T0 + 6), FdError::BadParams), "G2 for the jackpot");
    assert!(is_err(engine::set_starts(&mut c, 2 * SOL, START_MAX, T0 + 6), FdError::BadParams), "K2 for the jackpot (the pot holds 5 SOL)");
    assert!(is_err(engine::set_starts(&mut c, JP_MIN_POT - 1, START_MAX, T0 + 6), FdError::BadParams));
    assert!(is_err(engine::set_starts(&mut c, START_MAX + 1, START_MAX, T0 + 6), FdError::BadParams));
    c.mini_pool = 3 * SOL;
    assert!(is_err(engine::set_starts(&mut c, 4 * SOL, START_MAX, T0 + 6), FdError::BadParams), "G1: a decrease under an accumulated mini");
    c.mini_pool = SOL / 2;
    engine::set_starts(&mut c, 3 * SOL, START_MAX, T0 + 6).expect("a decrease within the bounds");

    // a super pot above twice its amount (not above START_MAX): the jackpot is free, the super can only be RAISED to ≥ pot / 2
    let mut c = cfg_with(100 * SOL, 5 * SOL, 20 * SOL, 0);
    c.super_pool = 50 * SOL;
    engine::set_starts(&mut c, 6 * SOL, 20 * SOL, T0 + 5).expect("the jackpot alone");
    assert!(is_err(engine::set_starts(&mut c, 5 * SOL, 24 * SOL, T0 + 5), FdError::BadParams), "24 < 50 / 2");
    assert!(is_err(engine::set_starts(&mut c, 5 * SOL, 19 * SOL, T0 + 5), FdError::BadParams));
    engine::set_starts(&mut c, 5 * SOL, 25 * SOL, T0 + 5).expect("a raise to half of the pot");
    assert!(is_err(engine::set_starts(&mut c, 5 * SOL, 200 * SOL + 1, T0 + 5), FdError::BadParams), "G2 from the pot: 4 × 50");

    // the mirror case: a jackpot pot above 2 × START_MAX does not stop a change of the super amount
    let mut c = cfg_with(100 * SOL, 5 * SOL, 20 * SOL, 0);
    c.jackpot_pool = 2 * START_MAX + 2;
    c.mini_pool = 900 * SOL;
    assert!(is_err(engine::set_starts(&mut c, START_MAX, 20 * SOL, T0 + 5), FdError::BadParams));
    assert!(is_err(engine::set_starts(&mut c, START_MAX, 80 * SOL, T0 + 5), FdError::BadParams), "an invalid jackpot change is not hidden by a valid super change");
    engine::set_starts(&mut c, 5 * SOL, 80 * SOL, T0 + 5).expect("the super alone");
    assert!(is_err(engine::set_starts(&mut c, 5 * SOL, 80 * SOL + 1, T0 + 5), FdError::BadParams), "G2 for the super");
    assert!(is_err(engine::set_starts(&mut c, 5 * SOL, 9 * SOL, T0 + 5), FdError::BadParams), "K2 for the super");
    assert!(is_err(engine::set_starts(&mut c, 5 * SOL, SUPER_MIN_POT - 1, T0 + 5), FdError::BadParams));

    // a command that changes neither amount is accepted in any state and only cancels a pending change
    let mut c = cfg_with(100 * SOL, 5 * SOL, 20 * SOL, 0);
    engine::set_starts(&mut c, 6 * SOL, 30 * SOL, T0 + 5).unwrap();
    c.jackpot_pool = 3 * START_MAX;
    c.super_pool = 3 * START_MAX;
    c.mini_pool = 3 * START_MAX;
    engine::set_starts(&mut c, 5 * SOL, 20 * SOL, T0 + 9).expect("nothing changes");
    assert_eq!((c.next_jackpot, c.next_super, c.starts_apply_ts), (5 * SOL, 20 * SOL, T0 + 9 + STARTS_DELAY_SECS));
    engine::apply_starts_if_due(&mut c, T0 + 9 + STARTS_DELAY_SECS);
    assert_eq!((c.init_jackpot, c.init_super), (5 * SOL, 20 * SOL));
}

/// The same as a property, on 300,000 random states: the command is accepted exactly when each amount, taken alone, is
/// either the one in effect or passes the bounds of ITS pot (written out here independently of engine.rs); the answer for
/// one amount never depends on the other pot. And the reason no start change can raise a player's expectation: with any
/// accepted amounts in effect, in any state of the pots, the pots are worth at most the constant of consts.rs per spin
/// (chance = contribution ÷ amount, win ≤ winner's share × pay cap × the same amount — the amount cancels).
#[test]
fn b_starts_are_checked_per_pot_and_never_raise_the_expectation() {
    let mut r = Sm(0xB0B5);
    let pick = |r: &mut Sm, base: u64| -> u64 {
        match r.below(9) {
            0 => 0,
            1 => base / 4,
            2 => base,
            3 => base * 2 - 1,
            4 => base * 2 + r.below(3),
            5 => base * 4 + r.below(3),
            6 => 2 * START_MAX + r.below(3),
            7 => r.below(4 * START_MAX),
            _ => r.below(base * 9 + 1),
        }
    };
    let amount = |r: &mut Sm, cur: u64, pool: u64, min: u64| -> u64 {
        match r.below(12) {
            0 | 1 | 2 => cur,
            3 => min - r.below(2),
            4 => START_MAX + r.below(2),
            5 => pool / 2 + r.below(2),
            6 => (pool / 2).saturating_sub(1),
            7 => cur.max(pool).saturating_mul(4).min(u64::MAX - 2) + r.below(2),
            8 => cur * 4 + r.below(2),
            9 => cur - r.below(cur / 2),
            10 => r.below(START_MAX + 2),
            _ => cur + r.below(3 * cur),
        }
    };
    // bound of the pots per executed spin, lamports × 2^32: consts.rs, pots_ev_bound_scaled (points × 10^6 × BPS²; a point is ONE lamports)
    let bound = pots_ev_bound_scaled() * ONE as u128 * (1u128 << 32) / 1_000_000 / (BPS as u128 * BPS as u128);
    let (mut accepted, mut one_changed, mut rejected) = (0u32, 0u32, 0u32);
    for _ in 0..300_000 {
        let j0 = [JP_MIN_POT, SOL, 5 * SOL, 250 * SOL, START_MAX][r.below(5) as usize];
        let s0 = [SUPER_MIN_POT, 20 * SOL, 320 * SOL, START_MAX][r.below(4) as usize];
        let mut c = cfg_with(100 * SOL, j0, s0, 0);
        c.jackpot_pool = pick(&mut r, j0);
        c.mini_pool = pick(&mut r, mini_start(j0));
        c.super_pool = pick(&mut r, s0);
        let (j, s) = (amount(&mut r, j0, c.jackpot_pool, JP_MIN_POT), amount(&mut r, s0, c.super_pool, SUPER_MIN_POT));
        let j_ok = j == j0
            || (j >= JP_MIN_POT && j <= START_MAX && j >= c.jackpot_pool / 2 && (j >= j0 || mini_start(j) >= c.mini_pool / 2) && j as u128 <= (c.jackpot_pool.max(j0) as u128 * 4).min(START_MAX as u128));
        let s_ok = s == s0 || (s >= SUPER_MIN_POT && s <= START_MAX && s >= c.super_pool / 2 && s as u128 <= (c.super_pool.max(s0) as u128 * 4).min(START_MAX as u128));
        let now = T0 + 50;
        let before = c.clone();
        let both = engine::set_starts(&mut c.clone(), j, s, now).is_ok();
        assert_eq!(both, j_ok && s_ok, "jackpot {j0} → {j} (pot {}, mini {}), super {s0} → {s} (pot {})", c.jackpot_pool, c.mini_pool, c.super_pool);
        // each amount alone, with the other left as it is: the other pot is not read
        assert_eq!(engine::set_starts(&mut c.clone(), j, s0, now).is_ok(), j_ok);
        assert_eq!(engine::set_starts(&mut c.clone(), j0, s, now).is_ok(), s_ok);
        let mut other = c.clone();
        other.super_pool = pick(&mut r, s0);
        assert_eq!(engine::set_starts(&mut other, j, s0, now).is_ok(), j_ok, "the jackpot answer does not depend on the super pot");
        let mut other = c.clone();
        other.jackpot_pool = pick(&mut r, j0);
        other.mini_pool = pick(&mut r, mini_start(j0));
        assert_eq!(engine::set_starts(&mut other, j0, s, now).is_ok(), s_ok, "the super answer does not depend on the jackpot and mini pots");
        if !both {
            rejected += 1;
            let mut d = c.clone();
            let _ = engine::set_starts(&mut d, j, s, now);
            assert_eq!((d.next_jackpot, d.next_super, d.starts_apply_ts, d.init_jackpot, d.init_super), (before.next_jackpot, before.next_super, before.starts_apply_ts, before.init_jackpot, before.init_super));
            continue;
        }
        accepted += 1;
        one_changed += ((j == j0) != (s == s0)) as u32;
        engine::set_starts(&mut c, j, s, now).unwrap();
        engine::apply_starts_if_due(&mut c, now + STARTS_DELAY_SECS);
        assert_eq!((c.init_jackpot, c.init_super), (j, s));
        // a bet placed now records (j, s); whatever the pots hold then, they are worth at most the bound per spin
        for mul in [1u64, 2, 3, 1_000] {
            let (jp, mp, sp) = (c.jackpot_pool.max(j).saturating_mul(mul), c.mini_pool.max(mini_start(j)).saturating_mul(mul), c.super_pool.max(s).saturating_mul(mul));
            let o = OddsBase::new_mode(j, s, 0).at_super(jp, mp, sp, true);
            let jw = (jp.min(pay_cap(jackpot_start(j), JP_PAY_CAP_BPS)) as u128 * JP_WINNER_BPS as u128 / BPS as u128) as u128;
            let mw = mp.min(pay_cap(mini_start(j), MINI_PAY_CAP_BPS)) as u128;
            let sw = sp.min(pay_cap(super_start(s), SUPER_PAY_CAP_BPS)) as u128 * SUPER_WINNER_BPS as u128 / BPS as u128;
            let ev = o.jackpot as u128 * jw + o.mini as u128 * mw + o.superj as u128 * sw;
            assert!(ev <= bound, "amounts {j}/{s}, pots ×{mul}: {ev} > {bound}");
        }
    }
    println!("set_starts on 300,000 random states: {accepted} accepted ({one_changed} of them change one amount only), {rejected} rejected; per-pot rule and the pots bound hold in all");
    assert!(accepted > 20_000 && rejected > 20_000 && one_changed > 5_000);
}

// =====================================================================================================================
// C. Withdrawal in regular SOL (withdraw_sol)
// =====================================================================================================================

/// Withdrawal accounting is one and the same function for both commands (`withdraw` and `withdraw_sol` call `engine::withdraw` and the same
/// vault check). Here are the rules the new command must preserve: no more than the balance, no fee, the player receives
/// exactly the requested amount; since change H there is no lock on what was deposited.
#[test]
fn c_withdraw_rules_shared_by_both_instructions() {
    let mut c = cfg_with(50 * SOL, 5 * SOL, 20 * SOL, 0);
    let mut p = player(&mut c, 1, T0);
    engine::deposit(&mut p, 2 * SOL, T0).unwrap();
    p.balance += 3 * SOL; // a win
    assert_eq!(engine::withdraw(&mut c, &mut p, 3 * SOL, T0 + 60).unwrap(), (3 * SOL, 0, 0));
    assert!(is_err(engine::withdraw(&mut c, &mut p, 0, T0 + 61).map(|_| ()), FdError::BadAmount));
    assert!(is_err(engine::withdraw(&mut c, &mut p, 2 * SOL + 1, T0 + 61).map(|_| ()), FdError::InsufficientBalance));
    assert_eq!(engine::withdraw(&mut c, &mut p, 2 * SOL, T0 + 61).unwrap(), (2 * SOL, 0, 0));
    assert_eq!(p.balance, 0);
    // size of the token account for which the command takes the rent deposit
    assert_eq!(TOKEN_ACCOUNT_LEN as usize, anchor_spl::token::TokenAccount::LEN);
}

// ---- Model of the path of the funds in withdraw_sol ----
// WARNING: this is a model, not a network. It repeats the rules of the token program and the system program as they are described
// in UPGRADE-2.md (section C) and checks the arithmetic and the order of the command's steps. The token program calls themselves are NOT
// executed on a network node by this test — the rehearsal on a local node does that.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Own {
    System,
    Token,
}
#[derive(Clone, Copy, Debug)]
struct Acc {
    lamports: u64,
    owner: Own,
    /// token account of wrapped SOL: Some(amount); system account: None
    amount: Option<u64>,
    /// rent deposit recorded in the token account at creation (amount = lamports − rent deposit)
    reserve: u64,
}
struct World {
    vault: Acc,
    unwrap: Acc,
    user: Acc,
}
impl World {
    fn total(&self) -> u64 {
        self.vault.lamports + self.unwrap.lamports + self.user.lamports
    }
}
/// Steps 1–4 of the command. An error at any step = the whole command is cancelled (the state does not change).
fn withdraw_sol_model(w0: &World, net: u64, rent_now: u64) -> Result<World, &'static str> {
    let mut w = World { vault: w0.vault, unwrap: w0.unwrap, user: w0.user };
    let user_before = w.user.lamports;
    // 1. token::transfer vault → service account (both are wrapped SOL accounts: the lamports move together with the amount)
    let va = w.vault.amount.ok_or("the vault is not a token account")?;
    if w.unwrap.owner != Own::Token || w.unwrap.amount.is_none() {
        return Err("the service account is not a token account");
    }
    if va < net {
        return Err("the vault holds less than the amount");
    }
    w.vault.amount = Some(va - net);
    w.vault.lamports -= net;
    w.unwrap.amount = Some(w.unwrap.amount.unwrap() + net);
    w.unwrap.lamports += net;
    // 2. token::close_account: a wrapped SOL account can be closed with a non-zero amount too; all lamports go to the recipient
    w.user.lamports += w.unwrap.lamports;
    w.unwrap = Acc { lamports: 0, owner: Own::System, amount: None, reserve: 0 };
    // 3. the player returns the rent deposit (the transfer is signed by the player), space and owner — by the signature of the service account's address
    let need = rent_now.saturating_sub(w.unwrap.lamports);
    if w.user.lamports < need {
        return Err("the player does not have enough for the rent deposit");
    }
    w.user.lamports -= need;
    w.unwrap.lamports += need;
    if w.unwrap.owner == Own::System {
        w.unwrap.owner = Own::Token;
    }
    // InitializeAccount3: the account must cover the rent deposit; the wrapped SOL amount = lamports − rent deposit
    if w.unwrap.lamports < rent_now {
        return Err("the rent deposit is not covered");
    }
    w.unwrap.reserve = rent_now;
    w.unwrap.amount = Some(w.unwrap.lamports - rent_now);
    // 4. the result check in the command itself
    if w.user.lamports < user_before + net {
        return Err("UnwrapFailed");
    }
    Ok(w)
}

#[test]
fn c_model_of_the_money_path() {
    const RENT: u64 = 2_039_280; // the network's rent deposit for a 165-byte token account (today's value)
    let mut r = Sm(0xC0FFEE);
    let (mut exact, mut gifts, mut refused) = (0u64, 0u64, 0u64);
    for _ in 0..200_000 {
        let vault_amount = 1 + r.below(1_000 * SOL);
        // the service account between commands: the rent deposit and a zero amount; occasionally someone has sent lamports or wrapped SOL to it
        let extra_tokens = if r.below(20) == 0 { r.below(SOL) } else { 0 };
        let extra_lamports_unsynced = if r.below(20) == 0 { r.below(SOL) } else { 0 };
        let rent_at_creation = RENT;
        let rent_now = match r.below(30) {
            0 => RENT / 2,       // the network lowered the rent deposit
            1 => RENT + 123_456, // the network raised the rent deposit
            _ => RENT,
        };
        let w = World {
            vault: Acc { lamports: RENT + vault_amount, owner: Own::Token, amount: Some(vault_amount), reserve: RENT },
            unwrap: Acc { lamports: rent_at_creation + extra_tokens + extra_lamports_unsynced, owner: Own::Token, amount: Some(extra_tokens), reserve: rent_at_creation },
            // the player has only enough for the network fee (it is charged before the command): from zero to a little
            user: Acc { lamports: r.below(20_000), owner: Own::System, amount: None, reserve: 0 },
        };
        let net = 1 + r.below(vault_amount);
        match withdraw_sol_model(&w, net, rent_now) {
            Ok(a) => {
                assert_eq!(a.total(), w.total(), "lamports are neither created nor destroyed");
                assert_eq!(a.vault.amount, Some(vault_amount - net), "exactly net left the vault");
                assert_eq!(a.vault.lamports, w.vault.lamports - net);
                assert_eq!((a.unwrap.owner, a.unwrap.amount, a.unwrap.lamports), (Own::Token, Some(0), rent_now), "the service account is empty and ready again");
                let gain = a.user.lamports - w.user.lamports;
                assert!(gain >= net);
                // above net the player receives only others' gifts to the service account and the rent difference, if the network lowered it
                assert_eq!(gain - net, extra_tokens + extra_lamports_unsynced + rent_at_creation - rent_now);
                if gain == net {
                    exact += 1;
                } else {
                    gifts += 1;
                }
            }
            Err(e) => {
                // the only reason for a rejection in the model: the network raised the rent deposit and the gifts on the account were not enough to cover it
                assert!(rent_now > rent_at_creation + extra_tokens + extra_lamports_unsynced, "{e}");
                assert!(e == "UnwrapFailed" || e == "the player does not have enough for the rent deposit");
                refused += 1;
            }
        }
    }
    println!("withdraw_sol model: exactly net — {exact}, net plus someone's gift/rent difference — {gifts}, rejection (the network raised the rent deposit) — {refused}");
    assert!(exact > 150_000 && gifts > 5_000 && refused > 1_000);
}

// =====================================================================================================================
// H. No withdrawal lock (owner's decision; the deployed v1 locks a deposit for 12 hours)
// =====================================================================================================================

fn owed(p: &Player) -> u64 {
    p.balance + p.jp_locked + if p.pending { p.p_stake } else { 0 }
}

/// A deposit can be withdrawn in full in the same second: both instructions use this one function.
#[test]
fn h_deposit_then_immediate_withdrawal_of_everything() {
    assert_eq!(DEPOSIT_LOCK_SECS, 0);
    let mut c = cfg_with(50 * SOL, 5 * SOL, 20 * SOL, 0);
    let pools0 = (c.prize_pool, c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_reserve, c.dev_pool, c.promo_pool);
    let mut p = player(&mut c, 1, T0);
    engine::deposit(&mut p, 7 * SOL, T0 + 5).unwrap();
    assert_eq!(engine::withdrawable(&p, T0 + 5), 7 * SOL);
    assert_eq!(engine::withdraw(&mut c, &mut p, 7 * SOL, T0 + 5).unwrap(), (7 * SOL, 0, 0), "the player receives exactly the deposit, no fee");
    assert_eq!((p.balance, p.dep_locked, owed(&p)), (0, 0, 0));
    // a deposit and a withdrawal moved nothing in the pools: an "in and out" costs the bank and the other players nothing
    assert_eq!(pools0, (c.prize_pool, c.jackpot_pool, c.mini_pool, c.super_pool, c.seed_reserve, c.dev_pool, c.promo_pool));
    assert!(is_err(engine::withdraw(&mut c, &mut p, 1, T0 + 5).map(|_| ()), FdError::InsufficientBalance));
}

/// With a round placed and not settled: the stake is already debited and stays owed to the round; only the rest leaves.
#[test]
fn h_withdrawal_with_an_open_round_takes_only_the_free_balance() {
    let mut c = cfg_with(50 * SOL, 5 * SOL, 20 * SOL, 0);
    let mut p = player(&mut c, 1, T0);
    engine::deposit(&mut p, 3 * SOL, T0 + 1).unwrap();
    engine::commit_mode(&mut c, &mut p, T0 + 1, 5, 100, 0, false, key(9)).unwrap();
    let stake = 100 * COST;
    assert_eq!((p.balance, p.p_stake, p.pending), (3 * SOL - stake, stake, true));
    assert!(is_err(engine::withdraw(&mut c, &mut p, 3 * SOL, T0 + 1).map(|_| ()), FdError::InsufficientBalance), "the stake of the open round cannot be withdrawn");
    assert_eq!(engine::withdraw(&mut c, &mut p, 3 * SOL - stake, T0 + 1).unwrap(), (3 * SOL - stake, 0, 0));
    assert_eq!((p.balance, owed(&p)), (0, stake), "the round is still owed its stake and can be settled or refunded");
    // the round is refunded (the house was silent): the stake comes back and can be withdrawn at once
    engine::refund(&mut c, &mut p, T0 + 2_000).unwrap();
    assert_eq!(engine::withdraw(&mut c, &mut p, stake, T0 + 2_000).unwrap(), (stake, 0, 0));
    assert_eq!(owed(&p), 0);
    // and a settled round: whatever it credited is free
    let mut q = player(&mut c, 2, T0);
    engine::deposit(&mut q, SOL, T0 + 3).unwrap();
    engine::commit_mode(&mut c, &mut q, T0 + 3, 6, 100, 0, false, key(9)).unwrap();
    engine::settle(&mut c, &mut q, Some([7u8; 32]), T0 + 4).unwrap();
    let all = q.balance;
    if all > 0 {
        assert_eq!(engine::withdraw(&mut c, &mut q, all, T0 + 4).unwrap(), (all, 0, 0));
    }
    assert_eq!(q.balance, 0);
}

/// An account written by the deployed program with a deposit frozen for 12 hours: after the upgrade the whole balance is free at once,
/// without a new deposit and without waiting for the recorded term.
#[test]
fn h_amount_frozen_before_the_upgrade_is_free_after_it() {
    let mut c = cfg_with(50 * SOL, 5 * SOL, 20 * SOL, 0);
    let mut p = player(&mut c, 1, T0);
    // what v1 wrote one minute before the upgrade: 5 SOL frozen until T0 + 12 h; part of it was lost since
    p.balance = 4 * SOL;
    p.dep_locked = 5 * SOL;
    p.last_deposit_ts = T0;
    p.dep_unlock_ts = T0 + 12 * 3_600;
    let now = T0 + 60;
    assert_eq!(engine::dep_locked_now(&p, now), 0);
    assert_eq!(engine::withdrawable(&p, now), 4 * SOL);
    assert_eq!(engine::withdraw(&mut c, &mut p, SOL, now).unwrap(), (SOL, 0, 0));
    assert_eq!(p.dep_locked, 0, "the stale frozen amount is cleared by the first withdrawal");
    // the same account topped up after the upgrade (also through the deposit address: deposit_sweep calls the same function)
    let mut q = player(&mut c, 2, T0);
    q.balance = 4 * SOL;
    q.dep_locked = 5 * SOL;
    q.dep_unlock_ts = T0 + 12 * 3_600;
    engine::deposit(&mut q, 2 * SOL, now).unwrap();
    assert_eq!((q.dep_locked, q.dep_unlock_ts), (2 * SOL, now), "the old frozen amount is not carried into the new record");
    assert_eq!(engine::withdraw(&mut c, &mut q, 6 * SOL, now).unwrap(), (6 * SOL, 0, 0));
    // extreme stored values do not overflow or underflow
    let mut r = player(&mut c, 3, T0);
    r.balance = SOL;
    r.dep_locked = u64::MAX;
    r.dep_unlock_ts = i64::MAX;
    assert_eq!(engine::withdrawable(&r, now), SOL);
    engine::deposit(&mut r, SOL, now).unwrap();
    assert_eq!(engine::withdraw(&mut c, &mut r, 2 * SOL, now).unwrap(), (2 * SOL, 0, 0));
    r.dep_unlock_ts = i64::MIN;
    engine::deposit(&mut r, SOL, now).unwrap();
    assert_eq!(engine::withdrawable(&r, now), SOL);
}

/// A frozen jackpot part (paid out by weeks) is a separate rule and did not change: it is not withdrawable early.
#[test]
fn h_jackpot_vesting_is_not_affected() {
    let mut c = cfg_with(50 * SOL, 5 * SOL, 20 * SOL, 0);
    let mut p = player(&mut c, 1, T0);
    engine::deposit(&mut p, SOL, T0).unwrap();
    p.jp_locked = 10 * SOL;
    p.jp_per_week = SOL;
    p.jp_clock = T0;
    assert_eq!(engine::withdrawable(&p, T0 + 1), SOL);
    assert!(is_err(engine::withdraw(&mut c, &mut p, SOL + 1, T0 + 1).map(|_| ()), FdError::InsufficientBalance));
}
