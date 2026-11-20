//! Tests of the pure game logic (no Solana). Run: `cargo test -p fairdrum --release -- --nocapture`

use anchor_lang::prelude::Pubkey;
use fairdrum::consts::*;
use fairdrum::engine;
use fairdrum::math::*;
use fairdrum::state::*;

const T0: i64 = 1_800_000_000;
const DAY: i64 = DAY_SECS;

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

const VAULT_AT_FUND: u64 = 25_000_000_000 * ONE + 10_000_000 * ONE + 5_000_000_000 * ONE + 10_000 * ONE + 1_000_000 * ONE;

fn funded_cfg() -> Config {
    let mut c = Config::default();
    c.start_ts = T0;
    c.init_act = 25_000_000_000 * ONE;
    c.init_prize = 10_000_000 * ONE;
    c.prize_buffer = 10_000_000 * ONE;
    c.init_seed_reserve = 5_000_000_000 * ONE;
    c.init_jackpot = 10_000 * ONE;
    c.init_super = 1_000_000 * ONE;
    engine::fund(&mut c, VAULT_AT_FUND).unwrap();
    c
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

// ---------- constants ----------
#[test]
fn split_sums_to_one_token() {
    let total = share(S_PRIZE) + share(S_JACK) + share(S_MINI) + share(S_SUPER) + share(S_DEV) + share(S_BURN) + share(S_ACT) + share(S_SEED);
    assert_eq!(total, COST);
    assert_eq!(LADDER.iter().map(|x| x.1).sum::<u64>(), LADDER_SUM);
    assert!(LADDER.iter().all(|x| x.0 >= 100), "there must be no small prizes");
    // the share of the former referrals now accumulates in the super jackpot
    assert_eq!((S_SUPER, S_DEV, S_ACT, S_BURN), (500, 884, 0, 0), "SOL version: 8.84% per spin to the administrator (≈10% of paid turnover), no burning");
    assert!(!BURN_ENABLED);
    assert_eq!(RANKS.map(|r| r.1), [0, 0, 0, 0, 0], "withdrawal without a fee");
    assert_eq!(JP_WINNER_BPS + JP_SUPER_BPS, BPS, "jackpot: 95% to the winner, 5% to the super jackpot");
    assert_eq!(COST, 10_000_000, "bet of 0.01 SOL in lamports");
}

#[test]
fn ranks_and_streaks() {
    assert_eq!(rank_idx(0), 0);
    assert_eq!(rank_idx(99), 0);
    assert_eq!(rank_idx(100), 1);
    assert_eq!(rank_idx(9_999), 2);
    assert_eq!(rank_idx(100_000), 4);
    assert_eq!(streak_bonus(6), 0);
    assert_eq!(streak_bonus(7), 0, "there are no free spins in the SOL version");
    assert_eq!(streak_bonus(90), 0);
    assert_eq!(batch_limit(0), 1_000, "batches of up to 1000 are open from day one (decision of 10.11.2026)");
    assert_eq!(batch_limit(9_999), 1_000);
    assert_eq!(batch_limit(1_000_000), 1_000);
}

// ---------- win plan: chances ----------
#[test]
fn win_plan_odds_scale_with_volume() {
    let ref_pot = |den: u64| (den as u128 * share(S_JACK) as u128) as u64; // average jackpot pot
    let mini_pot = |den: u64| (den as u128 / MINI_PER_JP as u128 * share(S_MINI) as u128) as u64;
    let q = |x: u64| x as f64 / 4_294_967_296.0;
    let start = odds(DEN_MIN, ref_pot(DEN_MIN), mini_pot(DEN_MIN));
    let big = odds(5_000_000, ref_pot(5_000_000), mini_pot(5_000_000));
    println!(
        "start (den {}): jackpot 1 in {:.0}, mini 1 in {:.0}, prize 1 in {:.0}",
        DEN_MIN,
        1.0 / q(start.jackpot),
        1.0 / q(start.mini),
        1.0 / q(start.prize)
    );
    println!("5 million a week: jackpot 1 in {:.0}, mini 1 in {:.0}", 1.0 / q(big.jackpot), 1.0 / q(big.mini));
    // at the beginning wins are more frequent
    assert!(start.jackpot > 1_000 * big.jackpot);
    assert!(start.mini > 1_000 * big.mini);
    // the chances of a prize and of "Again" do not depend on the volume
    assert_eq!(start.prize, big.prize);
    assert_eq!(start.again, big.again);
    // a jackpot roughly once per den×0.8 spins
    assert!((1.0 / q(start.jackpot) - DEN_MIN as f64 * 0.8).abs() < 1.0);
    assert!((q(start.mini) / q(start.jackpot) - MINI_PER_JP as f64).abs() < 0.01);
    // den below the minimum is treated as the minimum
    assert_eq!(odds(1, ref_pot(DEN_MIN), mini_pot(DEN_MIN)), start);
    // until the pot has accumulated 25% of the average, there is no chance of winning it
    let cold = odds(DEN_MIN, jackpot_min_pot(DEN_MIN) - 1, mini_min_pot(DEN_MIN) - 1);
    assert_eq!((cold.jackpot, cold.mini), (0, 0));
    let warm = odds(DEN_MIN, jackpot_min_pot(DEN_MIN), mini_min_pot(DEN_MIN));
    assert_eq!(warm, start);
    // gold in total remains a small share of spins at any volume, and "Again" + gold < 100%
    assert!(start.gold() + start.again < 1 << 32);
    println!("gold at the start: {:.2}% of spins", 100.0 * q(start.gold()));
}

#[test]
fn main_wheel_frequencies_at_start() {
    let o = odds(DEN_MIN, jackpot_min_pot(DEN_MIN), mini_min_pot(DEN_MIN));
    let mut r = Sm(1);
    let (mut e, mut a, mut g) = (0u64, 0u64, 0u64);
    let n = 4_000_000u64;
    for _ in 0..n {
        match draw_main(r.next() as u32, &o) {
            Main::Empty => e += 1,
            Main::Again => a += 1,
            Main::Gold => g += 1,
        }
    }
    let f = |x: u64| x as f64 / n as f64;
    let want_gold = o.gold() as f64 / 4_294_967_296.0;
    assert!((f(a) - 0.116).abs() < 0.002, "again {}", f(a));
    assert!((f(g) - want_gold).abs() < 0.0015, "gold {} vs {}", f(g), want_gold);
    assert!((f(e) - (1.0 - 0.116 - want_gold)).abs() < 0.003);
}

#[test]
fn gold_wheel_sectors_follow_odds_and_no_small_prizes() {
    let o = odds(DEN_MIN, jackpot_min_pot(DEN_MIN), mini_min_pot(DEN_MIN));
    let mut r = Sm(2);
    let n = 3_000_000u64;
    let (mut jp, mut mini, mut small) = (0u64, 0u64, 0u64);
    let mut by = std::collections::BTreeMap::new();
    for _ in 0..n {
        match draw_gold(r.next(), r.next() as u32, &o) {
            Gold::Super => panic!("the super jackpot is closed: there must be no chance"),
            Gold::Jackpot => jp += 1,
            Gold::Mini => mini += 1,
            Gold::Prize(t) => {
                if t < 1_000_000 {
                    small += 1
                }
                *by.entry(t).or_insert(0u64) += 1;
            }
        }
    }
    assert_eq!(small, 0);
    let total = o.gold() as f64;
    let f = |x: u64| x as f64 / n as f64;
    assert!((f(jp) - o.jackpot as f64 / total).abs() < 0.003, "jp {}", f(jp));
    assert!((f(mini) - o.mini as f64 / total).abs() < 0.003, "mini {}", f(mini));
    let p100 = *by.get(&1_000_000).unwrap() as f64 / n as f64;
    assert!((p100 - (o.prize as f64 / total) * 0.61).abs() < 0.004, "100: {}", p100);
    let ev: f64 = LADDER.iter().map(|(t, w)| *t as f64 * *w as f64).sum::<f64>() / LADDER_SUM as f64;
    assert!((ev - 2_410_000.0).abs() < 1e-9);
    // extreme values
    assert_eq!(draw_gold(0, 0, &o), Gold::Jackpot);
    assert_eq!(draw_gold(u64::MAX, 0, &o), Gold::Prize(1_000_000));
}

#[test]
fn rng_is_deterministic_and_seed_dependent() {
    let mut a = Rng::new([7; 32]);
    let mut b = Rng::new([7; 32]);
    let mut c = Rng::new([8; 32]);
    let va: Vec<u32> = (0..40).map(|_| a.next_u32()).collect();
    let vb: Vec<u32> = (0..40).map(|_| b.next_u32()).collect();
    let vc: Vec<u32> = (0..40).map(|_| c.next_u32()).collect();
    assert_eq!(va, vb);
    assert_ne!(va, vc);
    assert_ne!(Rng::new([7; 32]).gold(1), Rng::new([7; 32]).gold(2));
}

// ---------- launch and registration ----------
#[test]
fn funding_rules() {
    let mut c = Config::default();
    c.start_ts = T0;
    c.init_act = 100;
    c.init_prize = 10;
    c.prize_buffer = 10;
    c.init_seed_reserve = 5;
    c.init_jackpot = 1;
    assert!(engine::fund(&mut c, 115).is_err(), "not enough tokens");
    assert!(!c.funded);
    engine::fund(&mut c, 120).unwrap();
    assert!(c.funded);
    // the mini seed is taken from the seed reserve at funding (O21); here the reserve is tiny and goes entirely
    assert_eq!((c.act_pool, c.prize_pool, c.seed_reserve, c.jackpot_pool, c.mini_pool), (100, 10, 0, 1, 5));
    assert_eq!(c.promo_pool, 4, "the excess in the vault goes to the promotion fund");
    assert_eq!(c.den, DEN_MIN);
    assert!(engine::fund(&mut c, 120).is_err(), "not allowed a second time");
}

#[test]
fn mini_topup_only_fills_missing_seed() {
    let mut c = Config::default();
    c.start_ts = T0;
    c.init_act = 100;
    c.init_prize = 10;
    c.prize_buffer = 10;
    c.init_seed_reserve = 10 * MINI_SEED;
    c.init_jackpot = 1;
    assert!(engine::mini_topup(&mut c).is_err(), "not allowed before fund");
    engine::fund(&mut c, 111 + 10 * MINI_SEED).unwrap();
    assert_eq!(c.mini_pool, MINI_SEED, "fund places the seed itself");
    assert!(engine::mini_topup(&mut c).is_err(), "the mini is already seeded — nothing to top up");
    // state after fund() of the old program version (without O21): no seed, the reserve is intact
    c.seed_reserve += c.mini_pool;
    c.mini_pool = 0;
    let reserve = c.seed_reserve;
    assert_eq!(engine::mini_topup(&mut c).unwrap(), MINI_SEED);
    assert_eq!(c.mini_pool, MINI_SEED);
    assert_eq!(c.seed_reserve, reserve - MINI_SEED);
    assert!(engine::mini_topup(&mut c).is_err(), "not allowed again");
}

#[test]
fn rescale_moves_only_from_reserve() {
    let mut c = Config::default();
    c.start_ts = T0;
    c.init_act = 100;
    c.init_prize = 10;
    c.prize_buffer = 10;
    c.init_seed_reserve = MINI_SEED + 1_000; // after the mini seed exactly 1,000 will remain in the reserve
    c.init_jackpot = 1;
    assert!(engine::rescale(&mut c, 50, 10, 5, 5).is_err(), "not allowed before fund");
    engine::fund(&mut c, 111 + MINI_SEED + 1_000).unwrap();
    let reserve = c.seed_reserve;
    assert!(engine::rescale(&mut c, 0, reserve, 1, 0).is_err(), "more than the reserve is not allowed");
    let (p0, j0, s0) = (c.prize_pool, c.jackpot_pool, c.super_pool);
    engine::rescale(&mut c, 50, 10, 5, 7).unwrap();
    assert_eq!(c.prize_pool, p0 + 10);
    assert_eq!(c.jackpot_pool, j0 + 5);
    assert_eq!(c.super_pool, s0 + 7);
    assert_eq!(c.seed_reserve, reserve - 22);
    assert_eq!(c.prize_buffer, 50);
    engine::rescale(&mut c, 0, 1, 0, 0).unwrap();
    assert_eq!(c.prize_buffer, 50, "a zero buffer does not touch the previous one");
}

#[test]
fn registration_is_open_and_sequential_from_one() {
    let mut c = Config::default();
    assert!(engine::assign_number(&mut c).is_err(), "not allowed before funding");
    let mut c = funded_cfg();
    for want in 1..=5u64 {
        assert_eq!(engine::assign_number(&mut c).unwrap(), want);
    }
    let mut p = Player::default();
    engine::init_player(&mut p, key(1), 255, 6, T0);
    assert_eq!((p.balance, p.jp_locked, p.reg_no, p.streak), (0, 0, 6, 1), "no airdrop whatsoever");
}

#[test]
fn missing_a_day_burns_streak_and_rank_but_keeps_assets() {
    let mut c = funded_cfg();
    let t = T0 + DAY;
    let mut p = new_player(&mut c, 1, t);
    p.run_spins = 5_000;
    p.balance = 77 * ONE;
    let r = engine::touch(&mut p, t + DAY);
    assert!(!r.burned);
    assert_eq!(p.streak, 2);
    let r = engine::touch(&mut p, t + 3 * DAY + 5); // a whole day missed
    assert!(r.burned);
    assert_eq!((r.old_streak, r.old_run), (2, 5_000));
    assert_eq!((p.streak, p.run_spins), (1, 0));
    assert_eq!(p.balance, 77 * ONE, "the balance does not expire");
    assert_eq!(p.reg_no, 1);
    // for other players the expired rank is visible immediately
    let mut q = p.clone();
    q.run_spins = 20_000;
    assert_eq!(engine::effective_run(&q, day_of(t + 5 * DAY)), 0);
    assert_eq!(engine::effective_run(&q, day_of(t + DAY)), 20_000);
}

// ---------- spin ----------
#[test]
fn commit_validations() {
    let mut c = funded_cfg();
    let after = T0 + 5;
    let mut p = new_player(&mut c, 1, after);
    p.balance = 100 * COST;
    let r = Pubkey::new_unique();
    assert!(engine::commit(&mut c, &mut p, after, 10, 0, false, r).is_err(), "n = 0");
    assert!(engine::commit(&mut c, &mut p, after, 10, BATCH_LIMIT + 1, false, r).is_err(), "above the batch limit");
    assert!(engine::commit(&mut c, &mut p, T0 - 1, 10, 1, false, r).is_err(), "the game has not started");
    engine::commit(&mut c, &mut p, after, 10, 10, false, r).unwrap();
    assert_eq!(p.balance, 90 * COST);
    assert_eq!(p.p_stake, 10 * COST);
    assert!(engine::commit(&mut c, &mut p, after, 10, 1, false, r).is_err(), "while the spin is not finished");
    let mut empty = new_player(&mut c, 2, after);
    assert!(engine::commit(&mut c, &mut empty, after, 10, 1, false, r).is_err(), "no tokens");
}

#[test]
fn free_spins_are_disabled_in_sol_version() {
    // In SOL a free spin is a handout of value that bots would drain: the contract does not issue them at all
    let mut c = funded_cfg();
    let r = Pubkey::new_unique();
    let mut f = new_player(&mut c, 1, T0);
    assert_eq!(FREE_PER_WEEK, 0);
    assert!(engine::commit(&mut c, &mut f, T0, 5, 1, true, r).is_err());
    f.streak = 100;
    f.last_day = day_of(T0);
    assert!(engine::commit(&mut c, &mut f, T0, 6, 1, true, r).is_err(), "and a day streak gives no free ones");
}


#[test]
fn settle_conserves_tokens_with_many_players() {
    let mut c = funded_cfg();
    let after = T0 + 1;
    let mut rng = Sm(42);
    let mut players: Vec<Player> = vec![];
    for i in 0..30u64 {
        let mut p = new_player(&mut c, i, after);
        p.balance += 50_000 * COST; // deposit
        players.push(p);
    }
    let deposits = 30u128 * 50_000 * COST as u128;
    let base = VAULT_AT_FUND as u128;
    let mut now = after;
    for step in 0..6_000u64 {
        let i = (rng.next() % 30) as usize;
        let n = [1u16, 3, 10, 10][(rng.next() % 4) as usize];
        let free = step % 7 == 0;
        let r = Pubkey::new_unique();
        now += 600;
        if engine::commit(&mut c, &mut players[i], now, step + 1, n, free, r).is_err() {
            continue;
        }
        let seed = if step % 97 == 0 { None } else { Some(rng.seed()) }; // sometimes a spin expires by timeout
        now += 20;
        engine::settle(&mut c, &mut players[i], seed, now).unwrap();
        if step % 500 == 0 {
            let refs: Vec<&Player> = players.iter().collect();
            assert_eq!(ledger(&c, &refs) + c.total_burned as u128, base + deposits, "step {}", step);
        }
    }
    let refs: Vec<&Player> = players.iter().collect();
    assert_eq!(ledger(&c, &refs) + c.total_burned as u128, base + deposits);
    assert!(c.total_burned == 0 && c.total_spins > 0);
}

/// A run of one player at a given weekly volume: we count how often and how big they win
#[derive(Default, Debug)]
struct Plan {
    spins: u64,
    gold: u64,
    jp: u64,
    jp_sum: u64,
    mini: u64,
    mini_sum: u64,
    prizes: u64,
    spent: u64,
    won: u64,
    wins_within_10: u64,
    sessions: u64,
}

fn simulate(v_week: u64, weeks: u64, seed: u64) -> Plan {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 4_000_000_000 * ONE;
    let mut rng = Sm(seed);
    let step = (WEEK_SECS as u64 / v_week).max(1) as i64;
    let mut now = T0 + 1;
    let mut pl = Plan::default();
    let total = v_week * weeks;
    let mut k = 0u64;
    let mut session_hits = 0u64;
    // Warm-up: the first week and the first jackpot (it holds the starting 10,000 of seed) are not part of the measurement,
    // otherwise the average jackpot size is overstated several times. The start from zero itself is described in the comments to the test below.
    let mut warm = false;
    while pl.spins < total {
        if !warm && c.epoch_start > T0 && c.jackpots_won >= 1 {
            warm = true;
            pl = Plan::default();
            session_hits = 0;
        }
        now += step;
        k += 1;
        engine::commit(&mut c, &mut p, now, k, 1, false, Pubkey::new_unique()).unwrap();
        let b0 = p.balance;
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
        pl.spins += out.spins;
        pl.gold += out.gold as u64;
        pl.jp += out.jp_hits as u64;
        pl.jp_sum += out.jp_pool_total;
        pl.mini += out.mini_hits as u64;
        pl.mini_sum += out.mini_won;
        if out.prizes > 0 {
            pl.prizes += 1;
        }
        pl.spent += COST;
        pl.won += p.balance.saturating_sub(b0);
        // a "win" for the player: gold (prize, mini or jackpot)
        if out.gold > 0 {
            session_hits += 1;
        }
        if k % 10 == 0 {
            pl.sessions += 1;
            if session_hits > 0 {
                pl.wins_within_10 += 1;
            }
            session_hits = 0;
        }
    }
    pl
}

#[test]
fn win_plan_simulation_early_wins_are_frequent_and_small() {
    println!("\nweek ·  spins | jackpots (average, tok.) | mini (average) | gold once per | series of 10 with gold");
    let mut rows = vec![];
    for (v, weeks, seed) in [(500u64, 40u64, 11u64), (5_000, 12, 12), (50_000, 3, 13)] {
        let pl = simulate(v, weeks, seed);
        let jp_avg = if pl.jp > 0 { pl.jp_sum as f64 / pl.jp as f64 / ONE as f64 } else { 0.0 };
        let mini_avg = if pl.mini > 0 { pl.mini_sum as f64 / pl.mini as f64 / ONE as f64 } else { 0.0 };
        let gold_every = pl.spins as f64 / pl.gold.max(1) as f64;
        let any10 = pl.wins_within_10 as f64 / pl.sessions.max(1) as f64;
        println!(
            "{:>7} · {:>9} | {:>4} ({:>9.1}) | {:>5} ({:>7.1}) | {:>6.1} | {:.1}%",
            v, pl.spins, pl.jp, jp_avg, pl.mini, mini_avg, gold_every, 100.0 * any10
        );
        rows.push((v, jp_avg, mini_avg, gold_every, any10, pl));
    }
    // the fewer the players, the more frequent the gold and the smaller the wins
    assert!(rows[0].3 < rows[1].3 && rows[1].3 < rows[2].3, "gold is more frequent at the beginning: {:?}", rows.iter().map(|r| r.3).collect::<Vec<_>>());
    assert!(rows[0].2 < rows[1].2 && rows[1].2 < rows[2].2, "the mini is smaller at the beginning");
    assert!(rows[0].1 < rows[2].1, "the jackpot is smaller at the beginning");
    // at the start gold comes up roughly once in 50 spins, and every fifth series of 10 spins brings at least one
    assert!(rows[0].3 < 70.0, "gold once per {:.1}", rows[0].3);
    assert!(rows[0].4 > 0.12, "series of 10 with gold {:.3}", rows[0].4);
    // a jackpot at the start hits roughly once a week
    assert!(rows[0].5.jp >= 20 && rows[0].5.jp <= 70, "jackpots in 40 weeks {}", rows[0].5.jp);
    // and at a large volume also about one a week on average (volume ×100 — a jackpot ×100 bigger)
    assert!(rows[2].5.jp >= 1 && rows[2].5.jp <= 8, "jackpots in 3 weeks {}", rows[2].5.jp);
}

#[test]
fn return_to_player_is_stable() {
    // 0.6 million iterations of 10 spins ≈ 6.8 million spins at a volume of ≈ 12 thousand a week
    let mut c = funded_cfg();
    c.prize_pool = 1_000_000_000 * COST;
    c.prize_buffer = 1_000_000_000 * COST;
    c.super_pool = 0; // the starting seed of the super jackpot is not from bets: it is not part of the return to players
    c.seed_reserve = 0; // SOL version: no seed reserve, no subsidies from the owner's funds
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 10_000_000 * COST; // enough for 4 million spins given a return of ~90%
    let mut rng = Sm(99);
    let mut now = T0 + 1;
    let start_bal = p.balance;
    let iters = 400_000u64;
    let mut jackpots = 0u64;
    for k in 0..iters {
        now += 5_000;
        engine::commit(&mut c, &mut p, now, k + 1, 10, false, Pubkey::new_unique()).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
        jackpots += out.jp_hits as u64;
    }
    engine::touch(&mut p, now + 400 * DAY); // payout of the frozen part of jackpots
    assert_eq!(p.jp_locked, 0);
    let paid = iters as f64 * 10.0 * COST as f64;
    let back = (p.balance as f64 - start_bal as f64 + paid) / paid;
    let weeks = (now - T0) as f64 / WEEK_SECS as f64;
    println!("RTP = {:.4}, jackpots = {}, super jackpots = {} (once per {:.1} weeks), den = {}", back, jackpots, c.supers_won, weeks / c.supers_won.max(1) as f64, c.den);
    // new model: a constant chance of 1/SUPER_DEN per paid spin — the number of wins must
    // correspond to the number of spins (we allow a wide band: the sample is random)
    let expected = iters as f64 * 10.0 / SUPER_DEN as f64;
    assert!((c.supers_won as f64) > expected * 0.45 - 2.0 && (c.supers_won as f64) < expected * 1.7 + 2.0,
        "super jackpots {} with {:.1} expected", c.supers_won, expected);
    // mini reseeding (O21) is a deliberate subsidy from the seed reserve of ≈6.5% of a bet: the return is higher than before
    assert!(back > 0.80 && back < 0.96, "RTP {}", back);
    assert!(c.den >= 1_000, "the week's volume is accounted for: {}", c.den);
}

#[test]
fn forfeit_keeps_stake_in_pools() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 100 * COST;
    let before = ledger(&c, &[&p]);
    engine::commit(&mut c, &mut p, T0 + 1, 1, 10, false, Pubkey::new_unique()).unwrap();
    let out = engine::settle(&mut c, &mut p, None, T0 + 4_000).unwrap();
    assert!(out.forfeited && out.extras == 0 && out.prizes == 0);
    assert_eq!(p.balance, 90 * COST);
    assert_eq!(ledger(&c, &[&p]) + c.total_burned as u128, before);
    assert!(engine::settle(&mut c, &mut p, None, T0 + 4_001).is_err(), "not allowed a second time");
}


#[test]
fn jackpot_vesting_and_its_share_to_super_and_promo() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 1_000_000_000 * COST;
    let mut rng = Sm(5);
    let mut now = T0 + 1;
    let mut got = None;
    for k in 0..200_000u64 {
        now += 1_000;
        engine::commit(&mut c, &mut p, now, k + 1, 10, false, Pubkey::new_unique()).unwrap();
        let (sup0, promo0) = (c.super_pool, c.promo_pool);
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
        if out.jp_hits >= 1 {
            got = Some((out, sup0, promo0));
            break;
        }
    }
    let (out, sup0, promo0) = got.expect("a jackpot must hit");
    assert_eq!(out.jp_hits, 1, "the test expects one jackpot");
    let total = out.jp_pool_total;
    let win = total * 9_500 / 10_000;
    assert_eq!(out.jp_to_player_now, win * 2_000 / 10_000, "20% immediately");
    assert_eq!(p.jp_locked, win - win * 2_000 / 10_000);
    assert_eq!(p.jp_per_week, win * 800 / 10_000);
    // 5% of the jackpot goes to the super jackpot (plus its 5% from every spin), 5% stay with the promotion fund
    assert_eq!(c.super_pool - sup0, out.spins * share(S_SUPER) + total * 500 / 10_000, "5% of the jackpot to the super jackpot");
    assert_eq!(c.promo_pool - promo0, 0, "the promotion fund is not taken from the jackpot");
    // after 3 weeks 3 × 8% is paid out
    let locked0 = p.jp_locked;
    let b0 = p.balance;
    let t3 = p.jp_clock + 3 * WEEK_SECS;
    engine::release_jackpot(&mut p, t3);
    assert_eq!(p.balance - b0, 3 * (win * 800 / 10_000));
    assert_eq!(locked0 - p.jp_locked, 3 * (win * 800 / 10_000));
    let t20 = p.jp_clock + 20 * WEEK_SECS;
    engine::release_jackpot(&mut p, t20);
    assert_eq!(p.jp_locked, 0);
}

#[test]
fn admin_gets_its_share_of_every_executed_spin() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 1_000 * COST;
    let d0 = c.dev_pool;
    engine::commit(&mut c, &mut p, T0, 1, 10, false, Pubkey::new_unique()).unwrap();
    let out = engine::settle(&mut c, &mut p, Some([3; 32]), T0 + 5).unwrap();
    assert_eq!(c.dev_pool - d0, out.spins * share(S_DEV), "8.84% of every executed spin = 10% of paid turnover");
}

#[test]
fn again_spins_are_paid_by_prize_pool_and_bounded() {
    let mut c = funded_cfg();
    c.prize_pool = 0;
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 1_000 * COST;
    let mut rng = Sm(8);
    let mut extras = 0;
    for k in 0..3_000u64 {
        engine::commit(&mut c, &mut p, T0 + k as i64, k + 1, 10, false, Pubkey::new_unique()).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), T0 + k as i64 + 1).unwrap();
        assert!(out.spins <= 2 * 10 + 16, "'Again' cap");
        extras += out.extras;
        p.balance += 10 * COST;
    }
    assert!(extras > 0, "with an empty prize pool 'Again' is still possible thanks to the 35% inflow");
}

#[test]
fn withdraw_is_free_in_sol_version() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 10_000 * ONE;
    let j0 = c.jackpot_pool;
    let (net, fee, burn) = engine::withdraw(&mut c, &mut p, 1_000 * ONE, T0).unwrap();
    assert_eq!((net, fee, burn), (1_000 * ONE, 0, 0), "withdrawal without a fee and without burning");
    assert_eq!(c.jackpot_pool, j0);
    assert!(engine::withdraw(&mut c, &mut p, 1_000_000 * ONE, T0).is_err());
    assert!(engine::withdraw(&mut c, &mut p, 0, T0).is_err());
}

#[test]
fn epoch_roll_sets_den_and_moves_surplus() {
    let mut c = funded_cfg();
    c.prize_buffer = 1_000 * ONE;
    c.prize_pool = 5_000 * ONE;
    c.epoch_spins = 3_000_000;
    let j0 = c.jackpot_pool;
    assert!(engine::roll_epoch_if_due(&mut c, T0 + EPOCH_SECS - 1).is_none());
    let e = engine::roll_epoch_if_due(&mut c, T0 + EPOCH_SECS + 5).unwrap();
    assert_eq!(e.den, 3_000_000);
    assert_eq!(c.prize_pool, 1_000 * ONE);
    assert_eq!(c.jackpot_pool - j0, 4_000 * ONE);
    assert_eq!(c.epoch_spins, 0);
    // low turnover: the denominator hits the lower bound
    c.epoch_spins = 100;
    let t = c.epoch_start + EPOCH_SECS + 1;
    engine::roll_epoch_if_due(&mut c, t).unwrap();
    assert_eq!(c.den, DEN_MIN);
}

// =====================================================================================================
// Tests following the security self-review (docs/AUDIT.md)
// =====================================================================================================

/// The chances after the optimisation (OddsBase) match the direct formula they were computed by before
#[test]
fn odds_optimisation_matches_the_plain_formula() {
    fn reference(den: u64, jack: u64, mini: u64) -> Odds {
        let d = den.max(DEN_MIN) as u128;
        let q: u128 = 1 << 32;
        let base = q * BPS as u128 / (d * JP_GAP_BPS as u128);
        let jmin = (d * share(S_JACK) as u128 * COOLDOWN_BPS as u128 / BPS as u128) as u64;
        let mmin = (d / MINI_PER_JP as u128 * share(S_MINI) as u128 * COOLDOWN_BPS as u128 / BPS as u128) as u64;
        Odds {
            superj: 0,
            jackpot: if jack >= jmin { base as u64 } else { 0 },
            mini: if mini >= mmin { (base * MINI_PER_JP as u128) as u64 } else { 0 },
            prize: (q * P_PRIZE_PPM as u128 / 1_000_000) as u64,
            again: (q * W_AGAIN as u128 / BPS as u128) as u64,
        }
    }
    for den in [0u64, 1, 499, 500, 501, 777, 1_000, 12_345, 100_000, 5_000_000, u32::MAX as u64] {
        let (jmin, mmin) = (jackpot_min_pot(den), mini_min_pot(den));
        for jack in [0, jmin.saturating_sub(1), jmin, jmin + 1, 10_000 * ONE, u64::MAX / 4] {
            for mini in [0, mmin.saturating_sub(1), mmin, mmin + 1, 10_000 * ONE] {
                assert_eq!(odds(den, jack, mini), reference(den, jack, mini), "den {den} jack {jack} mini {mini}");
                assert_eq!(OddsBase::new(den).at(jack, mini), reference(den, jack, mini));
            }
        }
    }
}

/// Launch protection: the program data address is computed from the id of the program itself.
/// The reference pair is taken from the network: the Metaplex Token Metadata program and its ProgramData.
#[test]
fn initialize_is_bound_to_this_programs_own_code_account() {
    use std::str::FromStr;
    let meta = Pubkey::from_str("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s").unwrap();
    let meta_data = Pubkey::from_str("PwDiXFxQsGra4sFFTT8r1QWRMd4vfumiWC1jfWNfdYT").unwrap();
    assert_eq!(fairdrum::program_data_address(&meta), meta_data, "the address formula did not match the network");
    assert!(fairdrum::is_own_program_data(&meta, &meta_data));
    // another program's data cannot be slipped in
    let ours = Pubkey::new_unique();
    assert!(!fairdrum::is_own_program_data(&ours, &meta_data));
    assert!(!fairdrum::is_own_program_data(&ours, &ours));
    assert!(fairdrum::is_own_program_data(&ours, &fairdrum::program_data_address(&ours)));
}

/// Expiry of a spin (timeout) of a large batch is computed at once and exactly: nothing is won, everything goes into the pools
#[test]
fn forfeit_of_a_huge_batch_is_exact_and_needs_no_loop() {
    let mut c = funded_cfg();
    c.registered = 1_000_000; // allows a batch of 1000
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 5_000 * COST;
    let before = ledger(&c, &[&p]);
    let (j0, m0, s0, pr0, a0, d0, pp0, sup0) = (c.jackpot_pool, c.mini_pool, c.seed_pool, c.prize_pool, c.act_pool, c.dev_pool, c.promo_pool, c.super_pool);
    engine::commit(&mut c, &mut p, T0 + 1, 1, 1_000, false, Pubkey::new_unique()).unwrap();
    assert_eq!(p.balance, 4_000 * COST);
    let out = engine::settle(&mut c, &mut p, None, T0 + 4_000).unwrap();
    assert!(out.forfeited && out.spins == 1_000 && out.extras == 0 && out.gold == 0 && out.prizes == 0);
    assert_eq!(p.balance, 4_000 * COST, "nothing is returned on expiry");
    let n = 1_000u64;
    assert_eq!(c.jackpot_pool, j0 + n * share(S_JACK));
    assert_eq!(c.mini_pool, m0 + n * share(S_MINI));
    assert_eq!(c.seed_pool, s0 + n * share(S_SEED));
    assert_eq!(c.prize_pool, pr0 + n * share(S_PRIZE));
    assert_eq!(c.act_pool, a0 + n * share(S_ACT));
    assert_eq!(c.dev_pool, d0 + n * share(S_DEV));
    assert_eq!(c.super_pool, sup0 + n * share(S_SUPER), "the super jackpot's share goes into its pot");
    assert_eq!(c.promo_pool, pp0, "the promotion fund gets nothing on expiry");
    assert_eq!(out.burn, n * share(S_BURN));
    assert_eq!(c.total_spins, n);
    assert!(!p.pending);
    assert_eq!(ledger(&c, &[&p]) + out.burn as u128, before);
}

/// A batch of 1000 spins with real randomness also balances in tokens
#[test]
fn settled_batch_of_a_thousand_conserves_tokens() {
    let mut rng = Sm(9);
    for round in 0..20 {
        let mut c = funded_cfg();
        c.registered = 1_000_000;
        let mut p = new_player(&mut c, 1, T0);
        p.balance = 5_000 * COST;
        let before = ledger(&c, &[&p]);
        engine::commit(&mut c, &mut p, T0 + 1, 1, 1_000, false, Pubkey::new_unique()).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), T0 + 30).unwrap();
        assert!(out.spins >= 1_000 && out.spins <= 2 * 1_000 + 16, "round {round}");
        assert_eq!(ledger(&c, &[&p]) + out.burn as u128, before, "round {round}");
    }
}

/// The main check of honest accounting: whatever the players, the administrator and time do, the tokens in the vault
/// EXACTLY equal the sum of obligations (balances, frozen jackpot parts, pending bets, all pools).
/// Nothing is created out of thin air and nothing disappears.
#[test]
fn fuzz_every_operation_keeps_vault_exactly_equal_to_obligations() {
    let mut c = funded_cfg();
    c.registered = 1_000_000;
    let mut rng = Sm(2026);
    let mut vault: u128 = VAULT_AT_FUND as u128;
    let mut now = T0 + 1;
    let mut players: Vec<Player> = vec![];
    for i in 0..40u64 {
        players.push(new_player(&mut c, i, now));
    }
    let mut settled = 0u64;
    let mut forfeited = 0u64;
    let mut withdrawn = 0u128;
    for step in 0..40_000u64 {
        let jump = rng.next() % 100;
        now += if jump < 70 { 60 } else if jump < 90 { 3_600 } else if jump < 97 { DAY } else { 9 * DAY };
        let i = (rng.next() % 40) as usize;
        match rng.next() % 12 {
            0 => {
                let d = (rng.next() % 5_000 + 1) * COST;
                players[i].balance += d;
                vault += d as u128;
            }
            1 => {
                let bal = players[i].balance.max(1);
                let amount = 1 + rng.next() % bal;
                let mut p = std::mem::take(&mut players[i]);
                if let Ok((net, _fee, burn)) = engine::withdraw(&mut c, &mut p, amount, now) {
                    vault -= (net + burn) as u128;
                    withdrawn += net as u128;
                }
                players[i] = p;
            }
            2 => {
                let x = c.dev_pool / 2;
                c.dev_pool -= x;
                vault -= x as u128;
            }
            3 => {
                let x = c.promo_pool / 2;
                c.promo_pool -= x;
                vault -= x as u128;
            }
            4 => {
                let mut p = std::mem::take(&mut players[i]);
                engine::touch(&mut p, now);
                players[i] = p;
            }
            _ => {
                if players[i].pending {
                    let expire = rng.next() % 15 == 0;
                    let seed = if expire { None } else { Some(rng.seed()) };
                    let mut a = std::mem::take(&mut players[i]);
                    let out = engine::settle(&mut c, &mut a, seed, now).unwrap();
                    vault -= out.burn as u128;
                    forfeited += out.forfeited as u64;
                    settled += 1;
                    players[i] = a;
                } else {
                    let n = [1u16, 1, 2, 5, 10, 50, 100, 1000][(rng.next() % 8) as usize];
                    let free = rng.next() % 4 == 0;
                    let mut p = std::mem::take(&mut players[i]);
                    let _ = engine::commit(&mut c, &mut p, now, step + 1, n, free, Pubkey::new_unique());
                    players[i] = p;
                }
            }
        }
        let refs: Vec<&Player> = players.iter().collect();
        assert_eq!(ledger(&c, &refs), vault, "mismatch at step {step}");
    }
    assert!(settled > 1_000 && forfeited > 10 && withdrawn > 0);
    assert!(c.jackpots_won > 0 && c.minis_won > 0 && c.supers_won > 0, "the test must involve wins of all the jackpots: {} {} {}", c.jackpots_won, c.minis_won, c.supers_won);
    assert!(c.total_burned == 0 && c.dev_pool > 0);
}

/// Where paid bets go over a long run: the shares of the administrator and the promotion fund, the super jackpot, the burned part.
/// The figures from this test are quoted in docs/AUDIT.md and on the "Economy" page: the test keeps them from diverging from the code.
fn flows() -> (f64, f64, f64, f64, f64) {
    let mut c = funded_cfg();
    c.registered = 100_000; // batch of 100
    let mut rng = Sm(77);
    let mut p = new_player(&mut c, 501, T0);
    let (mut paid, mut now) = (0u64, T0 + 1);
    let mut supers_paid = 0u64;
    for step in 0..40_000u64 {
        now += 600;
        p.balance += 100 * COST; // we deposit exactly enough for a batch, so that all turnover is paid
        if engine::commit(&mut c, &mut p, now, step + 1, 100, false, Pubkey::new_unique()).is_err() {
            continue;
        }
        paid += 100;
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), now + 5).unwrap();
        supers_paid += out.super_total;
        // the player takes the winnings out so as not to play with them: only paid bets are in the turnover
        p.balance = 0;
    }
    let paid_t = (paid * COST) as f64;
    let sp = c.super_pool as f64;
    (c.dev_pool as f64 / paid_t, c.promo_pool as f64 / paid_t, (supers_paid as f64 + sp - 1_000_000.0 * ONE as f64) / paid_t, c.total_burned as f64 / paid_t, c.supers_won as f64)
}

#[test]
fn admin_share_is_ten_percent_of_paid_volume() {
    let (dev, promo, sup, burn, supers) = flows();
    eprintln!(
        "dev {:.2}% promo {:.2}% (together {:.2}%), {:.2}% came into the super jackpot (hit {} times), burned {:.2}%",
        dev * 100.0, promo * 100.0, (dev + promo) * 100.0, sup * 100.0, supers, burn * 100.0
    );
    // These bounds match the figures on the "Economy" page and in docs/AUDIT.md
    assert!(dev > 0.0985 && dev < 0.1015, "the administrator should receive 10% of paid turnover, not {dev}");
    assert!(promo == 0.0, "promo {promo}");
    assert!(sup > 0.05 && sup < 0.09, "super jackpot {sup}");
    assert!(burn == 0.0, "burn {burn}");
}


// =====================================================================================================
// Super jackpot: accumulates for a long time, hard to win
// =====================================================================================================

fn open_cfg() -> Config {
    // Enough weeks have passed since launch: the time lock is lifted
    funded_cfg()
}

/// There is no time lock any more: the super can hit from the very first day, provided the pot is not below the threshold
#[test]
fn super_jackpot_can_hit_from_day_one() {
    let mut c = open_cfg();
    c.registered = 1_000_000;
    c.super_pool = SUPER_MIN_POT * 3;
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 1_000_000_000 * COST;
    let mut rng = Sm(31);
    let mut now = T0 + 1; // right after the start
    let mut k = 0u64;
    while c.supers_won == 0 && k < 40_000 {
        now += 5;
        k += 1;
        engine::commit(&mut c, &mut p, now, k, 1000, false, Pubkey::new_unique()).unwrap();
        engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
    }
    assert_eq!(c.supers_won, 1, "with a chance of 1/{} per spin the super must hit long before {} spins", SUPER_DEN, k * 1000);
    assert_eq!(c.super_unlock_ts, c.start_ts, "the time lock does not appear after a win either");
}

/// Below the pot threshold there is no chance: however much you spin, the super does not hit
#[test]
fn super_jackpot_needs_min_pot() {
    let mut c = open_cfg();
    c.registered = 1_000_000;
    c.super_pool = 0;
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 4_000_000_000 * ONE;
    let mut rng = Sm(35);
    let mut now = T0 + 1;
    let mut k = 0u64;
    // we spin until the pot reaches the threshold: there must be no wins all this time
    while c.super_pool < SUPER_MIN_POT - 1000 * share(S_SUPER) && k < 500_000 {
        now += 5;
        k += 1;
        engine::commit(&mut c, &mut p, now, k, 1000, false, Pubkey::new_unique()).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
        assert_eq!(out.super_hits, 0, "the super cannot hit before the pot threshold");
    }
    assert!(c.supers_won == 0 && k > 10, "batches spun: {}", k);
}

/// Payout: 90% of the pot to the player (20% immediately, the rest week by week), 10% stays as the seed
#[test]
fn super_jackpot_pays_90_percent_and_keeps_10() {
    let mut c = open_cfg();
    c.registered = 1_000_000;
    c.super_pool = SUPER_MIN_POT * 5;
    let unlock_before = c.super_unlock_ts;
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 1_000_000_000 * COST;
    let mut rng = Sm(33);
    let mut now = T0 + 1;
    let mut found = None;
    for k in 0..400_000u64 {
        now += 600;
        let before = (c.super_pool, p.balance, p.jp_locked);
        engine::commit(&mut c, &mut p, now, k + 1, 1000, false, Pubkey::new_unique()).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
        if out.super_hits >= 1 {
            found = Some((out, before));
            break;
        }
    }
    let (out, before) = found.expect("the super jackpot must hit within 400 million spins");
    assert_eq!(out.super_hits, 1);
    assert!(c.super_pool > 0, "10% stays in the pot");
    let win = out.super_total;
    // after a win in the same batch the pot is replenished by the S_SUPER share of the remaining spins and 5% of the jackpots that hit
    let topup = out.jp_pool_total * JP_SUPER_BPS / BPS + 1000 * share(S_SUPER);
    assert!(win > 0 && c.super_pool <= win / 8 + topup, "the remainder is about 10% of the win: remainder {} with a win of {}", c.super_pool, win);
    assert!(c.super_pool >= win / 9, "the remainder is not less than 10%: {} with a win of {}", c.super_pool, win);
    assert_eq!(c.super_unlock_ts, unlock_before, "the old lock field does not change");
    assert_eq!(c.supers_won, 1);
    // 20% immediately, 80% is frozen and paid out week by week
    let now_part = win * 2_000 / 10_000;
    assert!(p.jp_locked - before.2 >= win - now_part - 1_000 && p.jp_locked - before.2 <= win - now_part + 1_000 || out.jp_hits > 0);
}

/// The chance and the threshold are constant and do not depend on turnover
#[test]
fn super_jackpot_odds_are_constant() {
    for den in [DEN_MIN, 1_000, 12_345, 1_000_000] {
        let ob = OddsBase::new(den);
        let closed_by_free = ob.at_super(0, 0, SUPER_MIN_POT * 100, false);
        let closed_by_pool = ob.at_super(0, 0, SUPER_MIN_POT - 1, true);
        let open = ob.at_super(0, 0, SUPER_MIN_POT, true);
        assert_eq!((closed_by_free.superj, closed_by_pool.superj), (0, 0));
        let want = (1u128 << 32) / SUPER_DEN as u128;
        assert_eq!(open.superj as u128, want, "den {den}: the chance does not depend on turnover");
    }
}

/// Mean expectation: about SUPER_DEN paid spins (checked on a sample)
#[test]
fn super_jackpot_mean_wait_is_about_super_den() {
    let mut rng = Sm(34);
    let trials = 60;
    let mut total_spins = 0u64;
    for _ in 0..trials {
        let mut c = open_cfg();
        c.registered = 1_000_000;
        c.super_pool = SUPER_MIN_POT * 10; // the threshold is certainly passed
        let mut p = new_player(&mut c, 1, T0);
        p.balance = 1_000_000_000 * COST;
        let mut now = T0 + 1;
        let mut k = 0u64;
        loop {
            now += 5;
            k += 1;
            engine::commit(&mut c, &mut p, now, k, 1000, false, Pubkey::new_unique()).unwrap();
            let out = engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
            total_spins += out.spins;
            if out.super_hits > 0 || k > 20_000 {
                break;
            }
        }
    }
    let mean = total_spins as f64 / trials as f64;
    let want = SUPER_DEN as f64;
    println!("average number of spins until the super jackpot: {:.0}, expected {:.0}", mean, want);
    assert!(mean > want * 0.7 && mean < want * 1.3, "{} versus {}", mean, want);
}

// ---------- game modes (11.11.2026): different outcome tables, equal expected payout ----------

/// Analytics: the expected payout of the prize ladder per spin is the same in every mode
/// (and none is more generous than the drum). This is the main invariant against a "profitable game".
#[test]
fn mode_evs_are_equal_and_not_above_wheel() {
    let ev = |m: usize| -> f64 {
        let s: u128 = MODE_LADDER[m].iter().map(|(t, w)| *t as u128 * *w as u128).sum();
        MODE_PPM[m] as f64 * (s as f64 / LADDER_SUM as f64) / 1_000_000.0
    };
    let ev0 = ev(0);
    println!("EV of mode 0 (drum): {:.2} tokens per spin", ev0);
    for m in 1..MODES {
        let e = ev(m);
        println!("EV of mode {}: {:.2} ({:+.3}%)", m, e, 100.0 * (e - ev0) / ev0);
        assert!(e <= ev0 + 1e-9, "mode {} is more generous than the drum: {} > {}", m, e, ev0);
        assert!(e >= ev0 * 0.995, "mode {} is poorer than the drum by more than 0.5%: {} < {}", m, e, ev0);
        let wsum: u64 = MODE_LADDER[m].iter().map(|(_, w)| w).sum();
        assert_eq!(wsum, LADDER_SUM, "the weights of mode {} do not add up to {}", m, LADDER_SUM);
    }
}

/// Packing the mode into p_n is reversible and does not touch the batch size
#[test]
fn pending_mode_packing_roundtrip() {
    for mode in 0..MODES as u8 {
        for n in [1u16, 2, 10, 100, 1_000] {
            let p = pack_pending(n, mode);
            assert_eq!(pending_n(p), n);
            assert_eq!(pending_mode(p), mode);
        }
    }
}

/// Batches larger than ×100 are allowed only for the drum; an unknown mode is rejected
#[test]
fn arcade_batches_are_capped_and_bad_mode_rejected() {
    let mut c = funded_cfg();
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 1_000_000 * COST;
    assert!(engine::commit_mode(&mut c, &mut p, T0 + 1, 1, 101, 1, false, Pubkey::new_unique()).is_err(), "arcade ×101 must be rejected");
    assert!(engine::commit_mode(&mut c, &mut p, T0 + 1, 1, 1, MODES as u8, false, Pubkey::new_unique()).is_err(), "unknown mode");
    engine::commit_mode(&mut c, &mut p, T0 + 1, 1, 100, 1, false, Pubkey::new_unique()).unwrap();
    engine::settle(&mut c, &mut p, None, T0 + 2).unwrap();
    engine::commit_mode(&mut c, &mut p, T0 + 3, 2, 1_000, 0, false, Pubkey::new_unique()).unwrap();
    engine::settle(&mut c, &mut p, None, T0 + 4).unwrap();
}

/// A mode's prizes come from that mode's ladder, and the frequency of "gold" in blackjack is orders of magnitude higher than on the drum
#[test]
fn mode_prizes_come_from_their_ladder() {
    let mut c = funded_cfg();
    c.prize_pool = 1_000_000_000 * COST;
    c.prize_buffer = c.prize_pool;
    let mut p = new_player(&mut c, 1, T0);
    p.balance = 2_000_000 * COST;
    let mut rng = Sm(7);
    let mut now = T0 + 1;
    let allowed: Vec<u64> = MODE_LADDER[1].iter().map(|(t, _)| t * ONE).collect();
    let (mut golds, mut spins) = (0u64, 0u64);
    for k in 0..12_000u64 {
        now += 500;
        engine::commit_mode(&mut c, &mut p, now, k + 1, 10, 1, false, Pubkey::new_unique()).unwrap();
        let out = engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
        golds += out.gold as u64;
        spins += out.spins;
        if out.best_prize > 0 {
            assert!(allowed.contains(&out.best_prize), "prize {} is not from the blackjack ladder", out.best_prize);
        }
    }
    let rate = golds as f64 / spins as f64;
    println!("blackjack: gold in {:.2}% of spins ({} of {})", rate * 100.0, golds, spins);
    // the mini/jackpot add ~0.3–0.5% to the prize chance — we take a wide band but check the order of magnitude
    assert!(rate > 0.05 && rate < 0.11, "blackjack gold frequency {:.4}", rate);
}

/// Simulation: the return to the player in an arcade mode is the same as on the drum (millions of spins)
#[test]
fn mode_rtp_matches_wheel() {
    let mut rtps = vec![];
    for (mode, seed) in [(0u8, 21u64), (1, 22), (3, 23), (7, 24)] {
        let mut c = funded_cfg();
        c.prize_pool = 1_000_000_000 * COST;
        c.prize_buffer = c.prize_pool;
        c.super_pool = 0;
        c.seed_reserve = 0; // SOL version: no subsidies from the reserve
        let mut p = new_player(&mut c, 1, T0);
        p.balance = 10_000_000 * COST;
        let mut rng = Sm(seed);
        let mut now = T0 + 1;
        let start = p.balance;
        let iters = 150_000u64;
        let n = if mode == 0 { 10u16 } else { 10u16.min(ARCADE_BATCH_LIMIT) };
        for k in 0..iters {
            now += 5_000;
            engine::commit_mode(&mut c, &mut p, now, k + 1, n, mode, false, Pubkey::new_unique()).unwrap();
            engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
        }
        engine::touch(&mut p, now + 400 * DAY);
        let paid = iters as f64 * n as f64 * COST as f64;
        let rtp = (p.balance as f64 - start as f64 + paid) / paid;
        println!("mode {}: RTP {:.4} (den {})", mode, rtp, c.den);
        rtps.push((mode, rtp));
        assert!(rtp > 0.80 && rtp < 0.97, "RTP of mode {} is outside the band: {}", mode, rtp);
    }
    let base = rtps[0].1;
    for (m, r) in &rtps[1..] {
        assert!((r - base).abs() < 0.05, "RTP of mode {} ({:.4}) diverged from the drum ({:.4})", m, r, base);
    }
}


// =====================================================================================================
// SOL version: launch with an EMPTY bank. The owner contributes nothing — all prizes come only from players' bets.

#[test]
fn sol_version_starts_from_empty_bank_and_never_owes_more_than_it_holds() {
    let mut c = Config::default();
    c.start_ts = T0;
    engine::fund(&mut c, 0).unwrap(); // all starting pots are zero
    let mut p = new_player(&mut c, 1, T0);
    let mut rng = Sm(2026);
    let mut now = T0 + 1;
    let (mut paid, mut taken) = (0u128, 0u128);
    for k in 0..300_000u64 {
        now += 300;
        p.balance += 10 * COST; // the player deposits exactly enough for a ×10 batch
        paid += 10 * COST as u128;
        engine::commit(&mut c, &mut p, now, k + 1, 10, false, Pubkey::new_unique()).unwrap();
        engine::settle(&mut c, &mut p, Some(rng.seed()), now + 1).unwrap();
        // takes out everything already available
        taken += p.balance as u128;
        p.balance = 0;
        // the bank always covers the obligations: deposited = paid out + pots + frozen payouts + administrator's share
        let held = c.prize_pool as u128 + c.jackpot_pool as u128 + c.mini_pool as u128 + c.super_pool as u128 + c.seed_pool as u128
            + c.seed_reserve as u128 + c.act_pool as u128 + c.dev_pool as u128 + c.promo_pool as u128 + p.jp_locked as u128;
        assert_eq!(paid, taken + held, "step {k}: funds neither appear nor disappear");
    }
    engine::touch(&mut p, now + 400 * DAY);
    taken += p.balance as u128;
    let rtp = taken as f64 / paid as f64;
    let admin = c.dev_pool as f64 / paid as f64;
    println!("SOL, empty bank: to players {:.2}%, to the administrator {:.2}%, in the pots {:.2}%", rtp * 100.0, admin * 100.0, (1.0 - rtp - admin) * 100.0);
    assert!(admin > 0.0985 && admin < 0.1015, "administrator {admin}");
    assert!(rtp > 0.84 && rtp < 0.90, "return to players {rtp}");
    assert_eq!(c.promo_pool + c.act_pool + c.total_burned, 0);
}
