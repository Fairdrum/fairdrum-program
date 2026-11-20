//! State carry-over between program versions (upgrade No. 2) and end-to-end accounting. Based on the tests of audit No. 9 ("program",
//! a9_common.rs): one and the same scenario is built both with the live sources and with the upgrade.
//! The model repeats the lib.rs wrappers (owed_of / track_owed / solvent) on top of the pure logic of engine.rs.
//!
//! The samples tests/xver/before.bin and after_cur.bin were written by the LIVE program version (sources before the upgrade,
//! test a9_xver_write): the accounts "before the upgrade" (400 players, each with an unfinished round, 153 of them below ×10) and the result
//! as computed by the live version. Test a9_xver_check reads these accounts with the upgraded program, completes the rounds and
//! requires a byte-for-byte match of every player account and of Config except the three numbers the economics change of 18.11.2026
//! moves (dev_pool, dev_total, super_pool: exactly 266,000 lamports per executed spin). How to rewrite the samples: docs/UPGRADE-2.md, section "Checks".
use anchor_lang::prelude::Pubkey;
use anchor_lang::{AccountDeserialize, AccountSerialize, Space};
use fairdrum::consts::*;
use fairdrum::engine;
use fairdrum::state::*;
mod v1ref;

const T0: i64 = 1_800_000_000;
const SOL: u64 = 1_000_000 * ONE;

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
fn mkey(i: u64) -> Pubkey {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&i.to_le_bytes());
    b[31] = 9;
    Pubkey::new_from_array(b)
}
// --- copy of the private functions of lib.rs ---
fn owed_of(p: &Player) -> u64 {
    p.balance.saturating_add(p.jp_locked).saturating_add(if p.pending { p.p_stake } else { 0 })
}
fn track(c: &mut Config, before: u64, after: u64) {
    c.owed = c.owed.checked_add(after).and_then(|x| x.checked_sub(before)).expect("track_owed");
}
fn pools(c: &Config) -> u64 {
    c.act_pool + c.prize_pool + c.jackpot_pool + c.mini_pool + c.super_pool + c.seed_pool + c.seed_reserve + c.dev_pool + c.promo_pool
}
fn solvent(c: &Config, vault: u64) -> bool {
    vault >= pools(c) + c.owed
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
    c.house_root = [1; 32];
    c.house_depth = 21;
    (c, vault)
}
fn player(c: &mut Config, i: u64, now: i64) -> Player {
    let n = engine::assign_number(c).unwrap();
    let mut p = Player::default();
    engine::init_player(&mut p, mkey(i), 255, n, now);
    p
}
/// spin_commit as in lib.rs (without signatures and session)
fn ix_commit(c: &mut Config, p: &mut Player, now: i64, slot: u64, n: u16, mode: u8) -> bool {
    if c.paused {
        return false;
    }
    let leaf = c.round_counter - c.house_start;
    if !(c.house_depth > 0 && leaf < (1u64 << c.house_depth)) {
        return false;
    }
    let before = owed_of(p);
    let (c0, p0) = (c.clone(), p.clone());
    if engine::commit_mode(c, p, now, slot, n, mode, false, Pubkey::new_from_array(c0.house_root)).is_err() {
        *c = c0;
        *p = p0;
        return false;
    }
    track(c, before, owed_of(p));
    p.p_leaf = leaf;
    p.p_sealed = false;
    p.p_salt = [0; 32];
    c.round_counter += 1;
    true
}

/// F invariant: after EVERY operation vault == pots + owed, and owed == the sum over the player accounts.
#[test]
fn a9_ledger_equality_under_random_sequences() {
    for (prize, jp, sp, res, tag) in [(SOL / 2, SOL / 5, 2 * SOL, 0, "bank 0.5 SOL with no reserve"), (50 * SOL, 5 * SOL, 20 * SOL, 30 * SOL, "bank 50 SOL")] {
        let (mut c, mut vault) = bank(prize, jp, sp, res);
        let mut r = Sm(0xA9);
        let mut now = T0 + 10;
        let mut slot = 1_000u64;
        let mut ps: Vec<Player> = (0..24).map(|i| player(&mut c, i, now)).collect();
        let (mut commits, mut settles, mut lates, mut refunds, mut wd, mut guard_trips) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
        for step in 0..250_000u64 {
            now += r.below(900) as i64;
            slot += 1 + r.below(2_000);
            let i = r.below(ps.len() as u64) as usize;
            match r.below(12) {
                0 | 1 => {
                    let amt = 1 + r.below(30 * SOL);
                    let b = owed_of(&ps[i]);
                    engine::deposit(&mut ps[i], amt, now).unwrap();
                    track(&mut c, b, owed_of(&ps[i]));
                    vault += amt;
                }
                2 | 3 | 4 => {
                    let mode = r.below(16) as u8;
                    let n = match r.below(6) {
                        0 => 1,
                        1 => 1 + r.below(9) as u16,
                        2 => 10,
                        3 => 100,
                        4 => 1000,
                        _ => 1 + r.below(1000) as u16,
                    };
                    if ix_commit(&mut c, &mut ps[i], now, slot, n, mode) {
                        commits += 1;
                    }
                }
                5 | 6 | 7 => {
                    if ps[i].pending {
                        let b = owed_of(&ps[i]);
                        match r.below(10) {
                            0 => {
                                engine::refund(&mut c, &mut ps[i], now).unwrap();
                                refunds += 1;
                            }
                            1 | 2 => {
                                engine::settle_late(&mut c, &mut ps[i], r.seed(), now).unwrap();
                                lates += 1;
                            }
                            _ => {
                                engine::settle(&mut c, &mut ps[i], Some(r.seed()), now).unwrap();
                                settles += 1;
                            }
                        }
                        track(&mut c, b, owed_of(&ps[i]));
                        if !solvent(&c, vault) {
                            guard_trips += 1;
                        }
                    }
                }
                8 => {
                    let can = engine::withdrawable(&ps[i], now);
                    if can > 0 {
                        let amt = if r.below(3) == 0 { can } else { 1 + r.below(can) };
                        let b = owed_of(&ps[i]);
                        let (c0, p0) = (c.clone(), ps[i].clone());
                        match engine::withdraw(&mut c, &mut ps[i], amt, now) {
                            Ok((net, fee, burn)) => {
                                assert_eq!((fee, burn), (0, 0));
                                track(&mut c, b, owed_of(&ps[i]));
                                assert!(solvent(&c, vault - net), "the vault protection triggered on a withdrawal, step {step}");
                                vault -= net;
                                wd += 1;
                            }
                            Err(_) => {
                                c = c0;
                                ps[i] = p0;
                            }
                        }
                    }
                }
                9 => {
                    // administrator actions: collect the share, move the reserve, change the starting amounts, donation
                    match r.below(5) {
                        0 if c.dev_pool > 0 => {
                            let a = 1 + r.below(c.dev_pool);
                            c.dev_pool -= a;
                            assert!(solvent(&c, vault - a));
                            vault -= a;
                        }
                        1 if c.promo_pool > 0 => {
                            let a = 1 + r.below(c.promo_pool);
                            c.promo_pool -= a;
                            vault -= a;
                        }
                        2 if c.seed_reserve > 0 => {
                            // accepted exactly while the bank ends at or below BANK_KEEP
                            let a = 1 + r.below(c.seed_reserve);
                            let fits = c.prize_pool as u128 + a as u128 <= BANK_KEEP as u128;
                            assert_eq!(engine::rescale(&mut c, a, 0, 0).is_ok(), fits);
                        }
                        3 => {
                            let _ = engine::set_starts(&mut c, JP_MIN_POT + r.below(20 * SOL), SUPER_MIN_POT + r.below(50 * SOL), now);
                        }
                        _ => {
                            let a = 1 + r.below(3 * SOL);
                            engine::donate(&mut c, a).unwrap();
                            vault += a;
                        }
                    }
                }
                10 => {
                    engine::touch(&mut ps[i], now);
                }
                _ => {
                    if ps.len() < 60 && r.below(50) == 0 {
                        ps.push(player(&mut c, 1000 + step, now));
                    }
                    // change of the secret root: numbering starts from the current bet (as in set_house_root)
                    if r.below(500) == 0 {
                        c.house_root = r.seed();
                        c.house_start = c.round_counter;
                    }
                }
            }
            let sum: u64 = ps.iter().map(owed_of).sum();
            assert_eq!(c.owed, sum, "owed diverged from the sum of the accounts, step {step}");
            assert_eq!(vault, pools(&c) + c.owed, "vault != pots + obligations, step {step}");
        }
        assert_eq!(guard_trips, 0);
        println!("[{tag}] bets {commits}, settlements {settles}, late {lates}, refunds {refunds}, withdrawals {wd}; vault {} = pots {} + owed {}", vault, pools(&c), c.owed);
    }
}

/// Audit No. 9 noted that the withdrawal-lock term was rounded up (someone else's smallest crediting shifted the term of a large
/// frozen amount by at least 1 second). Upgrade No. 2 removes the lock (change H): the same scenario now freezes nothing.
#[test]
fn a9_lock_ceil_step() {
    let (mut c, _) = bank(50 * SOL, 5 * SOL, 20 * SOL, 0);
    let mut p = player(&mut c, 1, T0);
    engine::deposit(&mut p, 10_000 * SOL, T0).unwrap();
    assert_eq!(DEPOSIT_LOCK_SECS, 0);
    let now = T0 + 1;
    let add = SWEEP_MIN - SWEEP_FEE_REFUND;
    for _ in 0..100 {
        engine::deposit(&mut p, add, now).unwrap();
    }
    assert_eq!(p.dep_unlock_ts, now);
    assert_eq!(engine::withdrawable(&p, now), 10_000 * SOL + 100 * add);
}

// ---------- state carry-over between versions: rounds not settled by the time of the upgrade ----------
fn xver_dir() -> Option<std::path::PathBuf> {
    // by default — the samples next to the test; A9_XVER_DIR sets another folder (for writing by the live version)
    Some(std::env::var("A9_XVER_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/xver")))
}
/// The same as xver_finish, but the round inputs are read before every settlement — as the settlement command does after
/// the upgrade (the RoundInputs event). Reading changes nothing: the result must match the live version.
fn xver_finish_new(c: &mut Config, ps: &mut [Player]) -> (u64, u64) {
    let mut r = Sm(0xF1A1);
    let now = T0 + 9 * 3_600;
    let (mut events, mut capped) = (0u64, 0u64);
    for (i, p) in ps.iter_mut().enumerate() {
        let b = owed_of(p);
        if p.pending {
            match i % 5 {
                0 => {
                    engine::refund(c, p, now).unwrap();
                }
                1 => {
                    let seed = r.seed();
                    let inp = engine::round_inputs(c, p, now, true);
                    assert!(inp.late && inp.n == pending_n(p.p_n) && inp.prize_pool == c.prize_pool);
                    events += 1;
                    engine::settle_late(c, p, seed, now).unwrap();
                }
                _ => {
                    let seed = r.seed();
                    let inp = engine::round_inputs(c, p, now, false);
                    assert!(!inp.late && inp.mode == pending_mode(p.p_n) && inp.prize_cap == p.p_prize_cap);
                    events += 1;
                    capped += (inp.prize_cap < 10 * SOL) as u64;
                    engine::settle(c, p, Some(seed), now).unwrap();
                }
            }
        }
        track(c, b, owed_of(p));
        let can = engine::withdrawable(p, now + 13 * 3_600);
        if can > 0 && i % 2 == 0 {
            let b = owed_of(p);
            engine::withdraw(c, p, can, now + 13 * 3_600).unwrap();
            track(c, b, owed_of(p));
        }
    }
    (events, capped)
}
fn ser(c: &Config, ps: &[Player]) -> Vec<u8> {
    let mut v = Vec::new();
    c.try_serialize(&mut v).unwrap();
    for p in ps {
        p.try_serialize(&mut v).unwrap();
    }
    v
}
fn de(b: &[u8], n: usize) -> (Config, Vec<Player>) {
    let (cs, psz) = (8 + Config::INIT_SPACE, 8 + Player::INIT_SPACE);
    let c = Config::try_deserialize(&mut &b[..cs]).unwrap();
    let ps = (0..n).map(|i| Player::try_deserialize(&mut &b[cs + i * psz..cs + (i + 1) * psz]).unwrap()).collect();
    (c, ps)
}
const XN: usize = 400;
/// We complete all unfinished rounds in three ways (timely settlement / late / refund), then withdraw.
fn xver_finish(c: &mut Config, ps: &mut [Player]) {
    let mut r = Sm(0xF1A1);
    let now = T0 + 9 * 3_600;
    for (i, p) in ps.iter_mut().enumerate() {
        let b = owed_of(p);
        if p.pending {
            match i % 5 {
                0 => {
                    engine::refund(c, p, now).unwrap();
                }
                1 => {
                    engine::settle_late(c, p, r.seed(), now).unwrap();
                }
                _ => {
                    engine::settle(c, p, Some(r.seed()), now).unwrap();
                }
            }
        }
        track(c, b, owed_of(p));
        let can = engine::withdrawable(p, now + 13 * 3_600);
        if can > 0 && i % 2 == 0 {
            let b = owed_of(p);
            engine::withdraw(c, p, can, now + 13 * 3_600).unwrap();
            track(c, b, owed_of(p));
        }
    }
}
/// Run on the LIVE version: writes the state "before the upgrade" and the result as the live version will compute it.
#[test]
fn a9_xver_write() {
    let Some(dir) = xver_dir() else { return };
    if std::env::var("A9_XVER_ROLE").as_deref() != Ok("write") {
        return;
    }
    let (mut c, _) = bank(SOL / 2, SOL / 5, 2 * SOL, SOL);
    let mut r = Sm(0x5EED);
    let mut ps: Vec<Player> = (0..XN as u64).map(|i| player(&mut c, i, T0 + 5)).collect();
    let mut now = T0 + 100;
    // history: deposits, rounds of all sizes (including ×1…×9, which can no longer be placed after the upgrade), settlements
    for round in 0..6 {
        for (i, p) in ps.iter_mut().enumerate() {
            now += 3;
            if !p.pending {
                let b = owed_of(p);
                engine::deposit(p, 12 * SOL, now).unwrap();
                track(&mut c, b, owed_of(p));
                let n = [1u16, 2, 9, 10, 37, 100, 500, 1000][(i + round) % 8];
                let mode = if n > 100 { 0 } else { (r.below(16)) as u8 };
                assert!(ix_commit(&mut c, p, now, 5_000 + now as u64, n, mode));
                p.p_sealed = i % 3 != 0;
                p.p_salt = r.seed();
            }
            if round < 5 && r.below(3) != 0 {
                let b = owed_of(p);
                engine::settle(&mut c, p, Some(r.seed()), now).unwrap();
                track(&mut c, b, owed_of(p));
            }
        }
    }
    let pend = ps.iter().filter(|p| p.pending).count();
    let small = ps.iter().filter(|p| p.pending && pending_n(p.p_n) < 10).count();
    std::fs::write(dir.join("before.bin"), ser(&c, &ps)).unwrap();
    xver_finish(&mut c, &mut ps);
    std::fs::write(dir.join("after_cur.bin"), ser(&c, &ps)).unwrap();
    println!("written: players {XN}, unfinished rounds {pend} (of them below ×10: {small}), bytes per player {}", 8 + Player::INIT_SPACE);
}
/// Run on the UPGRADE: reads the accounts "before the upgrade" byte for byte, completes the rounds and compares the result with the live version.
#[test]
fn a9_xver_check() {
    let Some(dir) = xver_dir() else { return };
    if std::env::var("A9_XVER_ROLE").as_deref() == Ok("write") {
        return;
    }
    let before = std::fs::read(dir.join("before.bin")).unwrap();
    assert_eq!(before.len(), 8 + Config::INIT_SPACE + XN * (8 + Player::INIT_SPACE), "the account sizes of this build are not those of the live one");
    let (mut c, mut ps) = de(&before, XN);
    assert_eq!(ser(&c, &ps), before, "reading and writing the accounts are not identical");
    let pend = ps.iter().filter(|p| p.pending).count();
    let small = ps.iter().filter(|p| p.pending && pending_n(p.p_n) < MIN_BATCH).count();
    assert!(pend == XN && small > 100, "the sample must contain unfinished rounds, including ones below the new minimum");
    // 1) with the same code as the live version
    let (mut c1, mut ps1) = (c.clone(), ps.clone());
    xver_finish(&mut c1, &mut ps1);
    // 2) with reading the round inputs before settlement, as in the upgraded settlement command
    let (events, capped) = xver_finish_new(&mut c, &mut ps);
    assert_eq!(ser(&c1, &ps1), ser(&c, &ps), "reading the round inputs changed the result");
    println!("round inputs read {events}, of them with a prize cap below the top prize {capped}; rounds below ×{MIN_BATCH}: {small}");
    let cur = std::fs::read(dir.join("after_cur.bin")).unwrap();
    assert!(ps.iter().all(|p| !p.pending));
    let (cs, psz) = (8 + Config::INIT_SPACE, 8 + Player::INIT_SPACE);
    let (c0, ps0) = de(&before, XN);
    let (cl, _) = de(&cur, XN);
    let differing = |mine: &[u8]| (0..XN).filter(|i| mine[cs + i * psz..cs + (i + 1) * psz] != cur[cs + i * psz..cs + (i + 1) * psz]).count();

    // (0) The v1 reference settlement (tests/v1ref: 884 to the administrator, 27 to the super jackpot, no pay caps, super topped
    // up from the reserve) with everything else taken from THIS program (refund, withdrawal, vesting, the draw, the tables)
    // reproduces the LIVE version's result byte for byte: Config and all 400 player accounts. So the reference is the live
    // settlement, and the rest of upgrade No. 2 changes nothing in a settlement.
    let (mut c1, mut ps1) = (c0.clone(), ps0.clone());
    xver_finish_v1(&mut c1, &mut ps1);
    assert!(ser(&c1, &ps1) == cur, "with the v1 economics the result differs from the live version: {} player accounts", differing(&ser(&c1, &ps1)));
    println!("v1 economics: matched the live version byte for byte: {} bytes, there were {pend} unfinished rounds", cur.len());

    // (1) The same rounds with the economics of the upgrade (they were placed under v1 at a 2 SOL super amount). The result is NOT
    // identical, and it must not be: the super jackpot sector is 293/27 times wider (6 super jackpots are won in this sample;
    // none with v1), and the two boundaries after it on the wheel move by that width. A draw that lands in a moved band becomes
    // a repeat or a gold spin, and with this sample's tiny bank (0.5 SOL: prizes are cut by the bank) one changed round changes
    // the cut of the next ones. The administrator's income is 618,000 lamports per executed spin.
    let mine = ser(&c, &ps);
    assert_eq!((c.dev_total - c0.dev_total) % share(S_DEV), 0);
    assert_eq!(c.dev_total - c0.dev_total, c.dev_pool - c0.dev_pool);
    println!("upgrade economics, rounds as placed under v1: {} of {XN} player accounts differ from the live result; super jackpots won {} (live: {})", differing(&mine), c.supers_won - c0.supers_won, cl.supers_won - c0.supers_won);
    assert!(differing(&mine) > 0 && c.supers_won > cl.supers_won);
}

/// xver_finish with the v1 reference settlement instead of the program's
fn xver_finish_v1(c: &mut Config, ps: &mut [Player]) {
    let mut r = Sm(0xF1A1);
    let now = T0 + 9 * 3_600;
    for (i, p) in ps.iter_mut().enumerate() {
        let b = owed_of(p);
        if p.pending {
            match i % 5 {
                0 => {
                    engine::refund(c, p, now).unwrap();
                }
                1 => {
                    v1ref::settle_v1(c, p, r.seed(), now, true);
                }
                _ => {
                    v1ref::settle_v1(c, p, r.seed(), now, false);
                }
            }
        }
        track(c, b, owed_of(p));
        let can = engine::withdrawable(p, now + 13 * 3_600);
        if can > 0 && i % 2 == 0 {
            let b = owed_of(p);
            engine::withdraw(c, p, can, now + 13 * 3_600).unwrap();
            track(c, b, owed_of(p));
        }
    }
}

#[test]
fn a9_sizes() {
    println!("Config {} bytes, Player {} bytes, Session {} bytes (with the 8-byte type discriminator)", 8 + Config::INIT_SPACE, 8 + Player::INIT_SPACE, 8 + Session::INIT_SPACE);
}
