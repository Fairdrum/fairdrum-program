//! Reference vectors for the public outcome re-computation (recompute.ts / verify.mjs).
//!
//! Every vector is produced by the UNMODIFIED engine (`engine::commit_mode` + `engine::settle` / `settle_late`):
//! the totals, the pool balances after settlement and the player's credit come straight from it.
//! The engine does not expose per-stake results, so the per-stake trace is produced by `trace()` below, which
//! is built only from the crate's public primitives (`Rng`, `OddsBase`, `draw_main`, `draw_gold_mode`) and is
//! asserted, for every single vector, to reproduce the engine's totals and final pool balances exactly.
//!
//! Upgrade 2: every vector also carries `event` — the fields of the `RoundInputs` event exactly as the program emits
//! them (`engine::round_inputs`, called right before the settlement like `finish_spin` in lib.rs does). The test asserts
//! that these fields equal the inputs the round was built from, including the two cases where Config holds a deferred
//! change of the start amounts (already due: the new amounts are in force; not yet due: the old ones are).
//! recompute-v2 builds its inputs from `event` + `value` only.
//!
//! Run: FD_VECTORS_OUT=/abs/path/vectors.json cargo test -p fairdrum --release --test vectors_gen -- --nocapture

use anchor_lang::prelude::Pubkey;
use fairdrum::consts::*;
use fairdrum::engine;
use fairdrum::math::*;
use fairdrum::state::*;
use std::fmt::Write as _;

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
    fn seed(&mut self) -> [u8; 32] {
        let mut s = [0u8; 32];
        for i in 0..4 {
            s[i * 8..i * 8 + 8].copy_from_slice(&self.next().to_le_bytes());
        }
        s
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u64) as usize]
    }
}

#[derive(Clone, Debug)]
struct Case {
    tag: &'static str,
    mode: u8,
    n: u16,
    free: bool,
    late: bool,
    seed: [u8; 32],
    // ---- fixed at commit ----
    /// prize bank at commit; the engine stores a quarter of it in Player.p_prize_cap
    commit_prize: u64,
    start_jackpot: u64,
    start_super: u64,
    // ---- Config at settlement ----
    prize: u64,
    jack: u64,
    mini: u64,
    sup: u64,
    seed_pool: u64,
    reserve: u64,
    init_jackpot: u64,
    init_super: u64,
}

impl Case {
    fn base(tag: &'static str, mode: u8, n: u16, seed: [u8; 32]) -> Case {
        Case {
            tag,
            mode,
            n,
            free: false,
            late: false,
            seed,
            commit_prize: 50 * SOL,
            start_jackpot: 5 * SOL,
            start_super: 20 * SOL,
            prize: 50 * SOL,
            jack: 5 * SOL,
            mini: SOL / 2,
            sup: 20 * SOL,
            seed_pool: 0,
            reserve: 100 * SOL,
            init_jackpot: 5 * SOL,
            init_super: 20 * SOL,
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq)]
struct Tot {
    spins: u64,
    extras: u64,
    gold: u32,
    prizes: u64,
    best_prize: u64,
    mini_hits: u32,
    mini_won: u64,
    jp_hits: u32,
    jp_total: u64,
    jp_now: u64,
    super_hits: u32,
    super_total: u64,
    /// added to the player's balance by this settlement
    credited: u64,
    /// added to the player's locked (weekly released) part
    locked: u64,
    per_week: u64,
    // pools after settlement
    prize: u64,
    jack: u64,
    mini: u64,
    sup: u64,
    seed_pool: u64,
    reserve: u64,
    promo: u64,
    /// moved from the prize bank to the seed reserve at the end of the settlement (already inside `prize` and `reserve`)
    released: u64,
}

/// What the program itself does with this round.
fn run_engine(c: &Case) -> (Tot, u64, engine::SettleInputs) {
    let mut cfg = Config::default();
    cfg.funded = true;
    cfg.start_ts = T0;
    cfg.epoch_start = T0;
    cfg.den = DEN_MIN;
    cfg.prize_pool = c.commit_prize;
    cfg.init_jackpot = c.start_jackpot;
    cfg.init_super = c.start_super;
    let mut p = Player::default();
    engine::init_player(&mut p, Pubkey::new_from_array([7; 32]), 255, 1, T0);
    let stake = c.n as u64 * COST;
    if c.free {
        // FREE_PER_WEEK = 0: the program accepts no free rounds today, so `commit_mode(free = true)` cannot succeed.
        // The pending fields are set exactly as commit_mode would set them.
        p.pending = true;
        p.p_n = pack_pending(c.n, c.mode);
        p.p_free = true;
        p.p_commit_slot = 1;
        p.p_stake = stake;
        p.p_prize_cap = cfg.prize_pool / PRIZE_BANK_DIV;
        p.p_start_jackpot = cfg.init_jackpot;
        p.p_start_super = cfg.init_super;
    } else {
        p.balance = stake;
        engine::commit_mode(&mut cfg, &mut p, T0 + 10, 1, c.n, c.mode, false, Pubkey::default()).unwrap();
        assert_eq!(p.balance, 0);
    }
    let cap = p.p_prize_cap;
    assert_eq!(cap, c.commit_prize / PRIZE_BANK_DIV);
    assert_eq!((p.p_start_jackpot, p.p_start_super), (c.start_jackpot, c.start_super));
    // the state other rounds and the administrator left in Config by the time of settlement
    cfg.prize_pool = c.prize;
    cfg.jackpot_pool = c.jack;
    cfg.mini_pool = c.mini;
    cfg.super_pool = c.sup;
    cfg.seed_pool = c.seed_pool;
    cfg.seed_reserve = c.reserve;
    cfg.init_jackpot = c.init_jackpot;
    cfg.init_super = c.init_super;
    // deferred change of the start amounts (set_jackpot_starts): the settlement applies it first if it is due
    match c.seed[0] % 4 {
        1 => {
            // due: Config still holds the old amounts, the new ones (those of the case) take force in this settlement
            cfg.init_jackpot = c.start_jackpot.max(JP_MIN_POT);
            cfg.init_super = c.start_super.max(SUPER_MIN_POT);
            cfg.next_jackpot = c.init_jackpot;
            cfg.next_super = c.init_super;
            cfg.starts_apply_ts = T0 + 15 + (c.seed[1] % 6) as i64; // T0+15 … T0+20 = the settlement time itself
        }
        2 => {
            // not due yet: the pending amounts must not be used
            cfg.next_jackpot = c.init_jackpot / 2 + 777;
            cfg.next_super = c.init_super * 2 + 1;
            cfg.starts_apply_ts = T0 + 21 + (c.seed[1] as i64) * 100;
        }
        _ => {}
    }
    let promo0 = cfg.promo_pool;
    let (bal0, lock0, pw0) = (p.balance, p.jp_locked, p.jp_per_week);
    // exactly what lib.rs::finish_spin publishes as the RoundInputs event, read before the settlement
    let ev = engine::round_inputs(&cfg, &p, T0 + 20, c.late);
    let out = if c.late { engine::settle_late(&mut cfg, &mut p, c.seed, T0 + 20).unwrap() } else { engine::settle(&mut cfg, &mut p, Some(c.seed), T0 + 20).unwrap() };
    assert!(!out.forfeited && !out.refunded && !p.pending);
    (
        Tot {
            spins: out.spins,
            extras: out.extras,
            gold: out.gold,
            prizes: out.prizes,
            best_prize: out.best_prize,
            mini_hits: out.mini_hits,
            mini_won: out.mini_won,
            jp_hits: out.jp_hits,
            jp_total: out.jp_pool_total,
            jp_now: out.jp_to_player_now,
            super_hits: out.super_hits,
            super_total: out.super_total,
            credited: p.balance - bal0,
            locked: p.jp_locked - lock0,
            per_week: p.jp_per_week - pw0,
            prize: cfg.prize_pool,
            jack: cfg.jackpot_pool,
            mini: cfg.mini_pool,
            sup: cfg.super_pool,
            seed_pool: cfg.seed_pool,
            reserve: cfg.seed_reserve,
            promo: cfg.promo_pool - promo0,
            released: out.bank_released,
        },
        cap,
        ev,
    )
}

fn bps(v: u64, b: u64) -> u64 {
    ((v as u128) * (b as u128) / (BPS as u128)) as u64
}

/// Per-stake results from the crate's public primitives. Codes, one per executed stake:
/// `.` nothing · `a` free re-spin granted · `A` free re-spin sector, not granted · `p<row>:<lamports>` table prize ·
/// `m:<lamports>` mini · `j:<pot>:<winner share>` jackpot · `s:<winner share>` super · `M` `J` `S` pot sector in a late round (pays nothing)
fn trace(c: &Case) -> (Tot, Vec<String>) {
    let n = c.n as u64;
    let cap_exec = 2 * n + 16;
    let prize_cap = c.commit_prize / PRIZE_BANK_DIV;
    let mut rng = Rng::new(c.seed);
    let ob = OddsBase::new_mode(c.start_jackpot, c.start_super, c.mode);
    let mut t = Tot { prize: c.prize, jack: c.jack, mini: c.mini, sup: c.sup, seed_pool: c.seed_pool, reserve: c.reserve, ..Default::default() };
    let mut codes = Vec::new();
    let (mut todo, mut exec) = (n, 0u64);
    while todo > 0 {
        todo -= 1;
        exec += 1;
        t.jack += share(S_JACK);
        t.mini += share(S_MINI);
        t.sup += share(S_SUPER);
        t.seed_pool += share(S_SEED);
        t.prize += share(S_PRIZE);
        let o = ob.at_super(t.jack, t.mini, t.sup, !c.free);
        match draw_main(rng.next_u32(), &o) {
            Main::Empty => codes.push(".".into()),
            Main::Again => {
                if exec < cap_exec && t.prize >= COST {
                    t.prize -= COST;
                    todo += 1;
                    t.extras += 1;
                    codes.push("a".into());
                } else {
                    codes.push("A".into());
                }
            }
            Main::Gold => {
                t.gold += 1;
                let (u1, u2) = rng.gold(exec as u32);
                match draw_gold_mode(u1, u2, &o, c.mode) {
                    Gold::Super if c.late => codes.push("S".into()),
                    Gold::Jackpot if c.late => codes.push("J".into()),
                    Gold::Mini if c.late => codes.push("M".into()),
                    Gold::Super => {
                        // upgrade No. 2: the win is computed from at most ×2.5 of the amount recorded at bet time; no top-up from the reserve
                        let base = t.sup.min((c.start_super.max(SUPER_MIN_POT) as u128 * SUPER_PAY_CAP_BPS as u128 / BPS as u128) as u64);
                        let win = bps(base, SUPER_WINNER_BPS);
                        t.sup -= win;
                        let now_part = bps(win, JP_NOW_BPS);
                        t.credited += now_part;
                        t.locked += win - now_part;
                        t.per_week += bps(win, JP_WEEKLY_BPS);
                        t.super_hits += 1;
                        t.super_total += win;
                        codes.push(format!("s:{}", win));
                    }
                    Gold::Jackpot => {
                        let total = t.jack.min((c.start_jackpot.max(JP_MIN_POT) as u128 * JP_PAY_CAP_BPS as u128 / BPS as u128) as u64);
                        // a fifth of the amount stays in the pot; the winner (95%) and the super jackpot (5%) share the rest
                        let stay = bps(total, JP_KEEP_BPS);
                        let win = bps(total - stay, JP_WINNER_BPS);
                        let to_super = bps(total - stay, JP_SUPER_BPS);
                        let total = win + to_super; // what leaves the pot
                        let keep = t.jack - total;
                        t.sup += to_super;
                        let now_part = bps(win, JP_NOW_BPS);
                        t.credited += now_part;
                        t.locked += win - now_part;
                        t.per_week += bps(win, JP_WEEKLY_BPS);
                        t.jp_now += now_part;
                        t.jp_hits += 1;
                        t.jp_total += total;
                        let top = c.init_jackpot.saturating_sub(keep + t.seed_pool).min(t.reserve);
                        t.jack = keep + t.seed_pool + top;
                        t.reserve -= top;
                        t.seed_pool = 0;
                        codes.push(format!("j:{}:{}", total, win));
                    }
                    Gold::Mini => {
                        let base = t.mini.min((mini_start(c.start_jackpot) as u128 * MINI_PAY_CAP_BPS as u128 / BPS as u128) as u64);
                        let win = base - bps(base, MINI_KEEP_BPS); // a third of the amount stays in the pot
                        let keep = t.mini - win;
                        t.credited += win;
                        t.mini_won += win;
                        t.mini_hits += 1;
                        let top = mini_start(c.init_jackpot).saturating_sub(keep).min(t.reserve);
                        t.mini = keep + top;
                        t.reserve -= top;
                        codes.push(format!("m:{}", win));
                    }
                    Gold::Prize(tokens) => {
                        // table row, found independently of draw_gold_mode and checked against it
                        let ladder = &MODE_LADDER[c.mode as usize];
                        let mut r = ((u2 as u64) * LADDER_SUM) >> 32;
                        let mut row = usize::MAX;
                        for (i, (_, w)) in ladder.iter().enumerate() {
                            if r < *w {
                                row = i;
                                break;
                            }
                            r -= *w;
                        }
                        assert!(row != usize::MAX && ladder[row].0 == tokens);
                        let amt = (tokens * ONE).min(t.prize / PRIZE_BANK_DIV).min(prize_cap);
                        t.prize -= amt;
                        t.credited += amt;
                        t.prizes += amt;
                        t.best_prize = t.best_prize.max(amt);
                        codes.push(format!("p{}:{}", row, amt));
                    }
                }
            }
        }
    }
    // the bank's surplus: what it holds above BANK_KEEP after the last stake, at most BANK_RELEASE_BPS of a bet per executed stake
    t.released = t.prize.saturating_sub(BANK_KEEP).min(exec * (COST / BPS * BANK_RELEASE_BPS));
    t.prize -= t.released;
    t.reserve += t.released;
    t.spins = exec;
    (t, codes)
}

/// The event exactly as lib.rs::finish_spin builds it from `engine::round_inputs`.
fn event_of(i: &engine::SettleInputs) -> RoundInputs {
    RoundInputs {
        owner: Pubkey::new_from_array([7; 32]),
        commit_slot: 1,
        mode: i.mode,
        n: i.n,
        free: i.free,
        late: i.late,
        prize_cap: i.prize_cap,
        start_jackpot: i.start_jackpot,
        start_super: i.start_super,
        prize_pool: i.prize_pool,
        jackpot_pool: i.jackpot_pool,
        mini_pool: i.mini_pool,
        super_pool: i.super_pool,
        seed_pool: i.seed_pool,
        seed_reserve: i.seed_reserve,
        init_jackpot: i.init_jackpot,
        init_super: i.init_super,
    }
}

fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{:02x}", b)).collect()
}

#[derive(Default)]
struct Stats {
    vectors: usize,
    stakes: u64,
    empty: u64,
    again: u64,
    again_denied: u64,
    prize: u64,
    prize_cut: u64,
    mini: u64,
    jackpot: u64,
    superj: u64,
    void_pot: u64,
    late: usize,
    free: usize,
    deferred_due: usize,
    deferred_pending: usize,
    rows: [[u64; 6]; MODES],
    by_mode: [usize; MODES],
}

struct Gen {
    out: String,
    st: Stats,
}

impl Gen {
    /// Runs the engine, checks the trace against it, writes the vector.
    fn emit(&mut self, c: &Case) {
        let (eng, cap, ev) = run_engine(c);
        // the event carries exactly the inputs of the round
        assert_eq!(
            ev,
            engine::SettleInputs {
                mode: c.mode,
                n: c.n,
                free: c.free,
                late: c.late,
                prize_cap: cap,
                start_jackpot: c.start_jackpot,
                start_super: c.start_super,
                prize_pool: c.prize,
                jackpot_pool: c.jack,
                mini_pool: c.mini,
                super_pool: c.sup,
                seed_pool: c.seed_pool,
                seed_reserve: c.reserve,
                init_jackpot: c.init_jackpot,
                init_super: c.init_super,
            },
            "RoundInputs differs from the inputs of the round: {:?}",
            c
        );
        self.st.deferred_due += (c.seed[0] % 4 == 1) as usize;
        self.st.deferred_pending += (c.seed[0] % 4 == 2) as usize;
        let (tr, codes) = trace(c);
        assert_eq!(eng, tr, "trace differs from the engine: {:?}", c);
        assert_eq!(codes.len() as u64, eng.spins);
        let s = &mut self.st;
        s.vectors += 1;
        s.stakes += eng.spins;
        s.late += c.late as usize;
        s.free += c.free as usize;
        s.by_mode[c.mode as usize] += 1;
        for code in &codes {
            match code.as_bytes()[0] {
                b'.' => s.empty += 1,
                b'a' => s.again += 1,
                b'A' => s.again_denied += 1,
                b'p' => {
                    s.prize += 1;
                    let row = (code.as_bytes()[1] - b'0') as usize;
                    s.rows[c.mode as usize][row] += 1;
                    let amt: u64 = code[3..].parse().unwrap();
                    if amt < MODE_LADDER[c.mode as usize][row].0 * ONE {
                        s.prize_cut += 1;
                    }
                }
                b'm' => s.mini += 1,
                b'j' => s.jackpot += 1,
                b's' => s.superj += 1,
                _ => s.void_pot += 1,
            }
        }
        if s.vectors > 1 {
            self.out.push_str(",\n");
        }
        write!(
            self.out,
            "{{\"tag\":\"{}\",\"in\":{{\"mode\":{},\"stakes\":{},\"free\":{},\"late\":{},\"value\":\"{}\",\"prizeCap\":\"{}\",\"startJackpot\":\"{}\",\"startSuper\":\"{}\",\"prizePool\":\"{}\",\"jackpotPool\":\"{}\",\"miniPool\":\"{}\",\"superPool\":\"{}\",\"seedPool\":\"{}\",\"seedReserve\":\"{}\",\"initJackpot\":\"{}\",\"initSuper\":\"{}\"}},",
            c.tag, c.mode, c.n, c.free, c.late, hex(&c.seed), cap, c.start_jackpot, c.start_super, c.prize, c.jack, c.mini, c.sup, c.seed_pool, c.reserve, c.init_jackpot, c.init_super
        )
        .unwrap();
        write!(
            self.out,
            "\"event\":{{\"commitSlot\":\"1\",\"mode\":{},\"n\":{},\"free\":{},\"late\":{},\"prizeCap\":\"{}\",\"startJackpot\":\"{}\",\"startSuper\":\"{}\",\"prizePool\":\"{}\",\"jackpotPool\":\"{}\",\"miniPool\":\"{}\",\"superPool\":\"{}\",\"seedPool\":\"{}\",\"seedReserve\":\"{}\",\"initJackpot\":\"{}\",\"initSuper\":\"{}\"}},",
            ev.mode, ev.n, ev.free, ev.late, ev.prize_cap, ev.start_jackpot, ev.start_super, ev.prize_pool, ev.jackpot_pool, ev.mini_pool, ev.super_pool, ev.seed_pool, ev.seed_reserve, ev.init_jackpot, ev.init_super
        )
        .unwrap();
        write!(
            self.out,
            "\"out\":{{\"spins\":{},\"extras\":{},\"gold\":{},\"prizes\":\"{}\",\"bestPrize\":\"{}\",\"miniHits\":{},\"miniWon\":\"{}\",\"jackpotHits\":{},\"jackpotTotal\":\"{}\",\"jackpotNow\":\"{}\",\"superHits\":{},\"superTotal\":\"{}\",\"credited\":\"{}\",\"locked\":\"{}\",\"perWeek\":\"{}\",\"prizePool\":\"{}\",\"jackpotPool\":\"{}\",\"miniPool\":\"{}\",\"superPool\":\"{}\",\"seedPool\":\"{}\",\"seedReserve\":\"{}\",\"promoAdded\":\"{}\",\"bankReleased\":\"{}\"}},",
            eng.spins, eng.extras, eng.gold, eng.prizes, eng.best_prize, eng.mini_hits, eng.mini_won, eng.jp_hits, eng.jp_total, eng.jp_now, eng.super_hits, eng.super_total, eng.credited, eng.locked, eng.per_week, eng.prize, eng.jack, eng.mini, eng.sup, eng.seed_pool, eng.reserve, eng.promo, eng.released
        )
        .unwrap();
        // every 25th vector also carries the event as the program serializes it (the bytes behind "Program data: …")
        if self.st.vectors % 25 == 1 {
            write!(self.out, "\"rawEvent\":\"{}\",", hex(&anchor_lang::Event::data(&event_of(&ev)))).unwrap();
        }
        write!(self.out, "\"trace\":\"{}\"}}", codes.join(",")).unwrap();
    }

    /// Tries seeds until the round satisfies `want`; emits it. Panics if nothing is found.
    fn search(&mut self, sm: &mut Sm, mut c: Case, max: u64, want: impl Fn(&Tot, &[String]) -> bool) {
        for _ in 0..max {
            c.seed = sm.seed();
            let (t, codes) = trace(&c);
            if want(&t, &codes) {
                self.emit(&c);
                return;
            }
        }
        panic!("no seed found for {:?}", c);
    }
}

impl Gen {
    /// Tries seeds until the two rounds (same seed, different inputs) give different per-stake results; emits both.
    fn search_diff(&mut self, sm: &mut Sm, mut a: Case, mut b: Case, max: u64) {
        for _ in 0..max {
            a.seed = sm.seed();
            b.seed = a.seed;
            if trace(&a).1 != trace(&b).1 {
                self.emit(&a);
                self.emit(&b);
                return;
            }
        }
        panic!("no differing seed found for {:?}", a);
    }
}

fn has(codes: &[String], first: u8) -> bool {
    codes.iter().any(|c| c.as_bytes()[0] == first)
}
fn count(codes: &[String], first: u8) -> usize {
    codes.iter().filter(|c| c.as_bytes()[0] == first).count()
}

#[test]
fn write_outcome_vectors() {
    let path = match std::env::var("FD_VECTORS_OUT") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("FD_VECTORS_OUT is not set: vectors are checked but not written");
            String::new()
        }
    };
    let mut g = Gen { out: String::new(), st: Stats::default() };
    let mut sm = Sm(0xFA1D_D2A5_2026_1115);

    // ---- A. grid: every mode × stake counts × free/paid × prize-bank situations × on time / late ----
    // (prize bank at commit, prize bank at settlement)
    let banks: [(&'static str, u64, u64); 6] = [
        ("grid/bank-large", 50 * SOL, 50 * SOL),
        ("grid/bank-tiny", SOL / 20, SOL / 20),
        ("grid/bank-zero", 0, 0),
        ("grid/bank-small-at-commit", SOL / 5, 30 * SOL),
        ("grid/bank-small-at-settle", 30 * SOL, SOL / 10),
        ("grid/bank-4sol", 4 * SOL, 4 * SOL),
    ];
    for mode in 0..MODES as u8 {
        for &n in &[1u16, 2, 3, 10, 37, 100] {
            for &free in &[false, true] {
                for &(tag, bc, bs) in &banks {
                    for &late in &[false, true] {
                        let mut c = Case::base(tag, mode, n, sm.seed());
                        c.free = free;
                        c.late = late;
                        c.commit_prize = bc;
                        c.prize = bs;
                        g.emit(&c);
                    }
                }
            }
        }
    }
    // large batches exist only on the wheel (mode 0)
    for &n in &[250u16, 500, 999, 1000] {
        for &free in &[false, true] {
            for &(tag, bc, bs) in &banks {
                for &late in &[false, true] {
                    let mut c = Case::base(tag, 0, n, sm.seed());
                    c.free = free;
                    c.late = late;
                    c.commit_prize = bc;
                    c.prize = bs;
                    g.emit(&c);
                }
            }
        }
    }

    // ---- B. random: everything varies, including closed pots, changed start amounts, empty reserve ----
    let starts_j = [0u64, JP_MIN_POT, JP_MIN_POT + 1, SOL, 5 * SOL, 37 * SOL + 12_345, START_MAX];
    let starts_s = [0u64, SUPER_MIN_POT, 3 * SOL + 7, 20 * SOL, 250 * SOL, START_MAX];
    let amounts = [0u64, 1, 9_999_999, COST, 12_345_678, SOL / 20, SOL / 4, SOL, 3 * SOL + 333, 40 * SOL, 1_000 * SOL, 123_456 * SOL + 789];
    for i in 0..700 {
        let mode = sm.below(MODES as u64) as u8;
        let n = if mode == 0 { sm.pick(&[1u16, 1, 2, 5, 10, 10, 64, 100, 100, 333, 1000]) } else { sm.pick(&[1u16, 1, 2, 5, 8, 9, 10, 16, 17, 50, 100]) };
        let mut c = Case::base("random", mode, n, sm.seed());
        c.free = sm.below(4) == 0;
        c.late = sm.below(5) == 0;
        c.commit_prize = sm.pick(&amounts);
        c.prize = sm.pick(&amounts);
        c.start_jackpot = sm.pick(&starts_j);
        c.start_super = sm.pick(&starts_s);
        // start amounts in Config at settlement: usually the same, sometimes already changed by the administrator
        c.init_jackpot = if i % 3 == 0 { sm.pick(&starts_j).max(JP_MIN_POT) } else { c.start_jackpot.max(JP_MIN_POT) };
        c.init_super = if i % 3 == 0 { sm.pick(&starts_s).max(SUPER_MIN_POT) } else { c.start_super.max(SUPER_MIN_POT) };
        // pots: from empty to several start amounts, with values right at the opening threshold
        let js = jackpot_start(c.start_jackpot);
        let ms = mini_start(c.start_jackpot);
        let ss = super_start(c.start_super);
        let near = |sm: &mut Sm, start: u64, sh: u64| -> u64 {
            let open = open_pot(start);
            match sm.below(8) {
                0 => 0,
                1 => open.saturating_sub(1),
                2 => open,
                3 => open.saturating_sub(sh),          // opens exactly on the first stake
                4 => open.saturating_sub(sh + 1),      // opens on the second stake
                5 => open.saturating_sub(sh * 3 + 5),
                6 => start,
                _ => start + sm.below(3 * start.min(1_000 * SOL) + 1),
            }
        };
        c.jack = near(&mut sm, js, share(S_JACK));
        c.mini = near(&mut sm, ms, share(S_MINI));
        c.sup = near(&mut sm, ss, share(S_SUPER));
        c.seed_pool = sm.pick(&[0u64, 0, 18_000, SOL / 3, 7 * SOL]);
        c.reserve = sm.pick(&[0u64, 0, 1, 5_000_000, SOL / 10, 2 * SOL, 100 * SOL]);
        g.emit(&c);
    }

    // ---- C. forced pot wins: seeds are searched until the pot is hit ----
    // smallest start amounts make the pots as likely as the program allows; pots are open and the reserve is varied
    for &mode in &[0u8, 1, 6, 11, 15] {
        for &n in &[1u16, 10, 100, 1000] {
            if mode != 0 && n > 100 {
                continue;
            }
            for &late in &[false, true] {
                for &(reserve, seed_pool) in &[(100 * SOL, 0u64), (0u64, 0u64), (SOL / 100, 40_000_000u64)] {
                    let mut c = Case::base("forced/mini", mode, n, [0; 32]);
                    c.late = late;
                    c.start_jackpot = JP_MIN_POT;
                    c.start_super = SUPER_MIN_POT;
                    c.init_jackpot = JP_MIN_POT;
                    c.init_super = SUPER_MIN_POT;
                    c.jack = 3 * JP_MIN_POT + 77;
                    c.mini = MINI_MIN_POT + 1_234_567;
                    c.sup = 2 * SUPER_MIN_POT + 999;
                    c.reserve = reserve;
                    c.seed_pool = seed_pool;
                    let (m, j, s) = if late { (b'M', b'J', b'S') } else { (b'm', b'j', b's') };
                    g.search(&mut sm, c.clone(), 50_000_000, |_, codes| has(codes, m));
                    c.tag = "forced/jackpot";
                    g.search(&mut sm, c.clone(), 50_000_000, |_, codes| has(codes, j));
                    c.tag = "forced/super";
                    g.search(&mut sm, c.clone(), 50_000_000, |_, codes| has(codes, s));
                }
            }
        }
    }
    // free rounds never draw the super pot, but do draw mini and jackpot
    for &n in &[1u16, 100, 1000] {
        let mut c = Case::base("forced/free-jackpot", 0, n, [0; 32]);
        c.free = true;
        c.start_jackpot = JP_MIN_POT;
        c.init_jackpot = JP_MIN_POT;
        c.start_super = SUPER_MIN_POT;
        c.init_super = SUPER_MIN_POT;
        g.search(&mut sm, c.clone(), 50_000_000, |_, codes| has(codes, b'j'));
        c.tag = "forced/free-mini";
        g.search(&mut sm, c.clone(), 50_000_000, |_, codes| has(codes, b'm'));
    }
    // several pots in one round: the refilled pot is drawn again, the jackpot feeds the super pot
    for k in 0..12 {
        let mut c = Case::base("forced/multi", 0, 1000, [0; 32]);
        c.start_jackpot = JP_MIN_POT;
        c.init_jackpot = if k % 2 == 0 { JP_MIN_POT } else { SOL };
        c.start_super = SUPER_MIN_POT;
        c.init_super = SUPER_MIN_POT;
        c.jack = JP_MIN_POT;
        c.mini = MINI_MIN_POT;
        c.sup = SUPER_MIN_POT;
        c.reserve = [100 * SOL, SOL / 4, 30_000_000, 0][k % 4];
        c.seed_pool = [0, 60_000_000][k % 2];
        g.search(&mut sm, c, 5_000_000, |_, codes| count(codes, b'm') >= 2 && count(codes, b'j') >= 1 + (k % 3 == 0) as usize);
    }
    for _ in 0..4 {
        let mut c = Case::base("forced/super+jackpot", 0, 1000, [0; 32]);
        c.start_jackpot = JP_MIN_POT;
        c.init_jackpot = JP_MIN_POT;
        c.start_super = SUPER_MIN_POT;
        c.init_super = SUPER_MIN_POT;
        c.reserve = SOL;
        g.search(&mut sm, c, 50_000_000, |_, codes| has(codes, b's') && has(codes, b'j'));
    }

    // ---- C2. upgrade No. 2: a pot win is computed from at most ×3 (jackpot, mini) / ×2.5 (super) of the amount recorded at bet time;
    // the rest stays in the pot. Pots exactly at the cap, one lamport above (after this stake's share) and far above; the amounts
    // in Config differ from those recorded at the bet (the cap follows the bet, the top-up follows Config).
    for k in 0..18usize {
        let pot = k % 3; // 0 jackpot, 1 mini, 2 super
        let mut c = Case::base(["forced/pay-cap-jackpot", "forced/pay-cap-mini", "forced/pay-cap-super"][pot], [0u8, 3, 15][k % 3], [1000u16, 100, 100][k % 3], [0; 32]);
        c.start_jackpot = JP_MIN_POT;
        c.start_super = SUPER_MIN_POT;
        c.init_jackpot = [JP_MIN_POT, SOL, 4 * JP_MIN_POT][(k / 3) % 3];
        c.init_super = [SUPER_MIN_POT, 8 * SOL][(k / 9) % 2];
        c.reserve = [100 * SOL, 0, SOL / 10][(k / 3) % 3];
        c.seed_pool = [0, 60_000_000][(k / 6) % 2];
        let over = [0u64, 1, 7 * SOL][(k / 3) % 3];
        let (cj, cm, cs) = (JP_MIN_POT * 3, MINI_MIN_POT * 3, SUPER_MIN_POT / 2 * 5);
        match pot {
            0 => c.jack = cj - share(S_JACK) + over,
            1 => c.mini = cm - share(S_MINI) + over,
            _ => c.sup = cs - share(S_SUPER) + over,
        }
        let first = [b'j', b'm', b's'][pot];
        g.search(&mut sm, c, 50_000_000, |t, codes| {
            // the win of the capped pot is the FIRST pot event of the round, so the pot is at or above the cap when it is won
            let i = codes.iter().position(|x| matches!(x.as_bytes()[0], b'j' | b'm' | b's'));
            let hit = i.map_or(false, |i| codes[i].as_bytes()[0] == first);
            hit && match pot {
                0 => t.jp_total >= cj,
                1 => t.mini_won >= cm,
                _ => t.super_total >= cs / 10 * 9,
            }
        });
    }

    // ---- D. forced table rows and the prize cap ----
    // every row of every table, paid in full (large bank)
    for mode in 0..MODES as u8 {
        for row in 0..6usize {
            if MODE_LADDER[mode as usize][row].1 == 0 {
                continue;
            }
            let c = Case::base("forced/row", mode, 10, [0; 32]);
            let full = MODE_LADDER[mode as usize][row].0 * ONE;
            let want = format!("p{}:{}", row, full);
            g.search(&mut sm, c, 200_000_000, |_, codes| codes.iter().any(|x| *x == want));
        }
    }
    // the cap from the bank at commit cuts the prize; the cap from the bank at settlement cuts the prize
    for mode in 0..MODES as u8 {
        let lad = MODE_LADDER[mode as usize];
        // a bank whose quarter is one lamport short of the third row: rows 2 and above are cut
        let small = (lad[2].0 * ONE - 1) * PRIZE_BANK_DIV;
        for &(tag, bc, bs) in &[("forced/cap-commit", small, 500 * SOL), ("forced/cap-settle", 500 * SOL, small), ("forced/cap-settle-shrinking", 500 * SOL, SOL / 8)] {
            let mut c = Case::base(tag, mode, 100, [0; 32]);
            c.commit_prize = bc;
            c.prize = bs;
            g.search(&mut sm, c, 1_000_000, |_, codes| {
                codes.iter().filter(|x| x.as_bytes()[0] == b'p').any(|x| {
                    let row = (x.as_bytes()[1] - b'0') as usize;
                    x[3..].parse::<u64>().unwrap() < lad[row].0 * ONE
                })
            });
        }
    }
    // free re-spin sector with a prize bank that cannot pay for it
    for mode in 0..MODES as u8 {
        for &n in &[1u16, 2] {
            let mut c = Case::base("forced/again-not-granted", mode, n, [0; 32]);
            c.prize = 0;
            c.commit_prize = if n == 1 { 0 } else { 40 * SOL };
            g.search(&mut sm, c, 1_000_000, |_, codes| has(codes, b'A'));
        }
    }
    // long chains of free re-spins
    for &n in &[1u16, 10] {
        let c = Case::base("forced/again-chain", 0, n, [0; 32]);
        g.search(&mut sm, c, 100_000_000, |t, _| t.extras >= if n == 1 { 6 } else { 9 });
    }
    // pots that open in the middle of the round (the gold threshold moves between stakes)
    for mode in 0..MODES as u8 {
        let mut c = Case::base("forced/pot-opens-midway", mode, 100, [0; 32]);
        c.start_jackpot = JP_MIN_POT;
        c.init_jackpot = JP_MIN_POT;
        c.start_super = SUPER_MIN_POT;
        c.init_super = SUPER_MIN_POT;
        c.jack = open_pot(JP_MIN_POT) - 30 * share(S_JACK);
        c.mini = open_pot(MINI_MIN_POT) - 20 * share(S_MINI);
        c.sup = open_pot(SUPER_MIN_POT) - 50 * share(S_SUPER);
        g.search(&mut sm, c, 10_000_000, |_, codes| has(codes, b'm'));
    }

    // ---- E. boundaries: pairs of rounds with the same round value whose results differ because of one input ----
    let minimal = |tag: &'static str, mode: u8, n: u16| {
        let mut c = Case::base(tag, mode, n, [0; 32]);
        c.start_jackpot = JP_MIN_POT;
        c.init_jackpot = JP_MIN_POT;
        c.start_super = SUPER_MIN_POT;
        c.init_super = SUPER_MIN_POT;
        c.jack = JP_MIN_POT;
        c.mini = MINI_MIN_POT;
        c.sup = SUPER_MIN_POT;
        c
    };
    for &mode in &[0u8, 4, 9, 15] {
        // free rounds do not draw the super pot: the thresholds differ from a paid round
        let paid = minimal("pair/free-vs-paid", mode, 100);
        let mut free = paid.clone();
        free.free = true;
        g.search_diff(&mut sm, paid, free, 50_000_000);
        // a pot is drawn from exactly a quarter of its start amount: one lamport less and it is not
        for pot in 0..3 {
            for &n in &[1u16, 3] {
                let mut open = minimal("pair/pot-threshold", mode, n);
                let mut shut = open.clone();
                match pot {
                    0 => {
                        open.jack = open_pot(JP_MIN_POT) - share(S_JACK);
                        shut.jack = open.jack - if n == 1 { 1 } else { 3 * share(S_JACK) };
                    }
                    1 => {
                        open.mini = open_pot(MINI_MIN_POT) - share(S_MINI);
                        shut.mini = open.mini - if n == 1 { 1 } else { 3 * share(S_MINI) };
                    }
                    _ => {
                        open.sup = open_pot(SUPER_MIN_POT) - share(S_SUPER);
                        shut.sup = open.sup - if n == 1 { 1 } else { 3 * share(S_SUPER) };
                    }
                }
                g.search_diff(&mut sm, open, shut, 200_000_000);
            }
        }
        // the odds follow the start amounts fixed at the stake, not the ones in force at settlement
        let a = minimal("pair/start-amounts", mode, 100);
        let mut b = a.clone();
        b.start_jackpot = 3 * JP_MIN_POT;
        b.start_super = 5 * SUPER_MIN_POT;
        g.search_diff(&mut sm, a, b, 50_000_000);
    }
    // refills after a pot win follow the start amounts in force at settlement
    for &(init_j, init_s, reserve, seed_pool, sup) in &[
        (3 * SOL, 7 * SOL, 100 * SOL, 0u64, SUPER_MIN_POT),
        (3 * SOL, 7 * SOL, SOL, 0, SUPER_MIN_POT),
        (3 * SOL, 7 * SOL, 100 * SOL, 4 * SOL, 100 * SOL),
        (JP_MIN_POT, SUPER_MIN_POT, 0, 4 * SOL, 100 * SOL),
        (40 * SOL, 900 * SOL, 500 * SOL, 123_456_789, 55 * SOL + 1),
    ] {
        for &n in &[10u16, 1000] {
            let mut c = minimal("forced/refill-super", 0, n);
            c.init_jackpot = init_j;
            c.init_super = init_s;
            c.reserve = reserve;
            c.seed_pool = seed_pool;
            c.sup = sup;
            g.search(&mut sm, c.clone(), 50_000_000, |_, codes| has(codes, b's'));
            c.tag = "forced/refill-jackpot";
            g.search(&mut sm, c.clone(), 50_000_000, |_, codes| has(codes, b'j'));
            c.tag = "forced/refill-mini";
            g.search(&mut sm, c.clone(), 50_000_000, |_, codes| has(codes, b'm'));
        }
    }

    // ---- C3. the bank's surplus goes to the seed reserve at the end of the settlement: nothing below BANK_KEEP, the whole
    // excess when it is small, at most 0.5% of a bet per executed stake when it is large; late rounds too.
    for (k, &over) in [-9 * SOL as i64, -1, 0, 1, 499_999, 30_000_000, 500 * SOL as i64, 100_000 * SOL as i64].iter().enumerate() {
        for &(mode, n, late) in &[(0u8, 10u16, false), (0, 1000, false), (5, 100, false), (9, 25, true)] {
            let mut c = Case::base("forced/bank-release", mode, n, sm.seed());
            c.late = late;
            c.prize = (BANK_KEEP as i64 + over) as u64;
            c.commit_prize = if k % 2 == 0 { c.prize } else { c.prize / 2 };
            c.reserve = [0, 3 * SOL][k % 2];
            g.emit(&c);
        }
    }

    // ---- header: the program's tables and constants, so ports can check their copy ----
    let mut tables = String::new();
    for m in 0..MODES {
        if m > 0 {
            tables.push(',');
        }
        write!(tables, "{{\"ppm\":{},\"ladder\":[", MODE_PPM[m]).unwrap();
        for (i, (tok, w)) in MODE_LADDER[m].iter().enumerate() {
            if i > 0 {
                tables.push(',');
            }
            write!(tables, "[{},{}]", tok * ONE, w).unwrap();
        }
        tables.push_str("]}");
    }
    let secret = [0x11u8; 32];
    let salt = [0x22u8; 32];
    let consts = format!(
        "{{\"cost\":{},\"bps\":{},\"sPrize\":{},\"sJack\":{},\"sMini\":{},\"sSuper\":{},\"sSeed\":{},\"againBps\":{},\"jpMinPot\":{},\"miniMinPot\":{},\"miniStartDiv\":{},\"superMinPot\":{},\"cooldownBps\":{},\"prizeBankDiv\":{},\"ladderSum\":{},\"jpWinnerBps\":{},\"jpSuperBps\":{},\"jpNowBps\":{},\"jpWeeklyBps\":{},\"superWinnerBps\":{},\"jpPayCapBps\":{},\"miniPayCapBps\":{},\"superPayCapBps\":{},\"superTopUp\":false,\"jpKeepBps\":{},\"miniKeepBps\":{},\"bankKeep\":{},\"bankReleaseBps\":{},\"expirySlots\":{}}}",
        COST, BPS, S_PRIZE, S_JACK, S_MINI, S_SUPER, S_SEED, W_AGAIN, JP_MIN_POT, MINI_MIN_POT, MINI_START_DIV, SUPER_MIN_POT, COOLDOWN_BPS, PRIZE_BANK_DIV, LADDER_SUM, JP_WINNER_BPS, JP_SUPER_BPS, JP_NOW_BPS, JP_WEEKLY_BPS, SUPER_WINNER_BPS, JP_PAY_CAP_BPS, MINI_PAY_CAP_BPS, SUPER_PAY_CAP_BPS, JP_KEEP_BPS, MINI_KEEP_BPS, BANK_KEEP, BANK_RELEASE_BPS, PENDING_EXPIRY_SLOTS
    );
    // ---- boundary values of the two draw primitives (a hash that lands exactly on a boundary cannot be searched for) ----
    let mut prim = String::new();
    let mut prim_n = 0;
    for mode in 0..MODES as u8 {
        for &(sj, ss) in &[(0u64, 0u64), (5 * SOL, 20 * SOL), (START_MAX, START_MAX)] {
            let ob = OddsBase::new_mode(sj, ss, mode);
            for mask in 0..8u8 {
                let big = u64::MAX / 2;
                let o = ob.at_super(if mask & 1 != 0 { big } else { 0 }, if mask & 2 != 0 { big } else { 0 }, big, mask & 4 != 0);
                let gold = o.gold();
                let mut us: Vec<u32> = vec![0, 1, u32::MAX, u32::MAX - 1, (gold - 1) as u32, gold as u32, (gold + 1) as u32, (gold + o.again - 1) as u32, (gold + o.again) as u32, (gold + o.again + 1) as u32];
                for _ in 0..4 {
                    us.push(sm.next() as u32);
                }
                let main: Vec<String> = us.iter().map(|&u| format!("[{},\"{}\"]", u, match draw_main(u, &o) { Main::Gold => "gold", Main::Again => "again", Main::Empty => "nothing" })).collect();
                // u1: around each pot boundary ceil(q·2^64 / total); u2: around each row boundary ceil(c·2^32 / 10000)
                let total = gold as u128;
                let mut u1s: Vec<u64> = vec![0, 1, u64::MAX, u64::MAX - 1];
                for q in [o.superj, o.superj + o.jackpot, o.superj + o.jackpot + o.mini] {
                    let b = (((q as u128) << 64) + total - 1) / total;
                    for d in [-2i128, -1, 0, 1] {
                        let v = b as i128 + d;
                        if v >= 0 && v <= u64::MAX as i128 {
                            u1s.push(v as u64);
                        }
                    }
                }
                let mut u2s: Vec<u32> = vec![0, 1, u32::MAX, u32::MAX - 1];
                let mut c = 0u64;
                for (_, w) in MODE_LADDER[mode as usize].iter() {
                    c += w;
                    let b = ((c << 32) + LADDER_SUM - 1) / LADDER_SUM;
                    for d in [-1i64, 0, 1] {
                        let v = b as i64 + d;
                        if v >= 0 && v <= u32::MAX as i64 {
                            u2s.push(v as u32);
                        }
                    }
                }
                let mut gold_cases: Vec<String> = Vec::new();
                for (k, &u1) in u1s.iter().enumerate() {
                    // every u1 with a few u2, and the last u1 (a table prize) with every u2
                    let pick: Vec<u32> = if k + 1 == u1s.len() || k == 2 { u2s.clone() } else { vec![u2s[k % u2s.len()], sm.next() as u32] };
                    for u2 in pick {
                        let r = match draw_gold_mode(u1, u2, &o, mode) {
                            Gold::Super => "S".to_string(),
                            Gold::Jackpot => "J".to_string(),
                            Gold::Mini => "M".to_string(),
                            Gold::Prize(tok) => {
                                let lad = &MODE_LADDER[mode as usize];
                                let mut r = ((u2 as u64) * LADDER_SUM) >> 32;
                                let mut row = 9;
                                for (i, (_, w)) in lad.iter().enumerate() {
                                    if r < *w {
                                        row = i;
                                        break;
                                    }
                                    r -= *w;
                                }
                                assert_eq!(lad[row].0, tok);
                                format!("{}", row)
                            }
                        };
                        gold_cases.push(format!("[\"{}\",{},\"{}\"]", u1, u2, r));
                    }
                }
                if prim_n > 0 {
                    prim.push_str(",\n");
                }
                prim_n += main.len() + gold_cases.len();
                write!(prim, "{{\"mode\":{},\"odds\":[\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"],\"main\":[{}],\"gold\":[{}]}}", mode, o.superj, o.jackpot, o.mini, o.prize, o.again, main.join(","), gold_cases.join(",")).unwrap();
            }
        }
    }
    println!("primitive boundary cases: {}", prim_n);

    // ---- one whole settlement transaction as its three events: RoundRevealed, RoundInputs, SpinSettled ----
    // The round value is derived from the secret and the salt (math::salted), as on chain. A small bank: prizes are cut.
    let sample_tx = {
        let mut sc = Case::base("sample-tx", 0, 100, salted(secret, &salt));
        sc.commit_prize = SOL / 5;
        sc.prize = SOL / 3;
        sc.reserve = SOL;
        let (eng, _, ev) = run_engine(&sc);
        let cut0 = g.st.prize_cut;
        g.emit(&sc);
        assert!(g.st.prize_cut > cut0, "the sample transaction must contain a prize cut by the cap");
        let owner = Pubkey::new_from_array([7; 32]);
        let rr = RoundRevealed { owner, leaf: 5, root: [9; 32], secret, salt, commit_slot: 1 };
        let ss = SpinSettled {
            owner,
            spins: eng.spins,
            extras: eng.extras,
            gold: eng.gold,
            prizes: eng.prizes,
            best_prize: eng.best_prize,
            mini_hits: eng.mini_hits,
            mini_won: eng.mini_won,
            jackpot_hits: eng.jp_hits,
            jackpot_total: eng.jp_total,
            super_hits: eng.super_hits,
            super_total: eng.super_total,
            forfeited: false,
            refunded: false,
        };
        format!(
            "{{\"owner\":\"{}\",\"roundRevealed\":\"{}\",\"roundInputs\":\"{}\",\"spinSettled\":\"{}\"}}",
            owner,
            hex(&anchor_lang::Event::data(&rr)),
            hex(&anchor_lang::Event::data(&event_of(&ev))),
            hex(&anchor_lang::Event::data(&ss))
        )
    };
    let json = format!(
        "{{\"source\":\"fairdrum engine (engine::settle, engine::settle_late), tests/vectors_gen.rs\",\n\"consts\":{},\n\"modes\":[{}],\n\"salted\":{{\"secret\":\"{}\",\"salt\":\"{}\",\"value\":\"{}\"}},\n\"sampleTx\":{},\n\"primitives\":[\n{}\n],\n\"vectors\":[\n{}\n]}}\n",
        consts,
        tables,
        hex(&secret),
        hex(&salt),
        hex(&salted(secret, &salt)),
        sample_tx,
        prim,
        g.out
    );
    let s = &g.st;
    println!("vectors: {}  stakes executed: {}", s.vectors, s.stakes);
    println!("late: {}  free: {}", s.late, s.free);
    println!("deferred start amounts: due at settlement {}  not yet due {}", s.deferred_due, s.deferred_pending);
    println!("nothing: {}  again granted: {}  again not granted: {}", s.empty, s.again, s.again_denied);
    println!("table prizes: {} (cut by the cap: {})", s.prize, s.prize_cut);
    println!("mini: {}  jackpot: {}  super: {}  pot sectors in late rounds: {}", s.mini, s.jackpot, s.superj, s.void_pot);
    println!("vectors per mode: {:?}", s.by_mode);
    for m in 0..MODES {
        println!("mode {:2} rows hit: {:?}", m, s.rows[m]);
        for row in 0..6 {
            assert!(s.rows[m][row] > 0 || MODE_LADDER[m][row].1 == 0, "row never hit");
        }
    }
    assert!(s.vectors >= 2_000);
    if !path.is_empty() {
        std::fs::write(&path, json).unwrap();
        println!("written: {}", path);
    }
}
