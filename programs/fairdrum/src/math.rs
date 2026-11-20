//! Pure math with no Solana calls: randomness, chances (the "win plan"), the draw, ranks.

use crate::consts::*;
use solana_sha256_hasher::hashv;

// ---------------- stream of random numbers from the 32-byte round number ----------------

/// From a single 32-byte value (unpredictable and unavailable to the player before the commit) we derive as many
/// numbers as needed via sha256. This is safe: the player does not influence the seed, and sha256 cannot be "ground".
pub struct Rng {
    seed: [u8; 32],
    block: u32,
    buf: [u8; 32],
    pos: usize,
}

impl Rng {
    pub fn new(seed: [u8; 32]) -> Self {
        let mut r = Rng { seed, block: 0, buf: [0; 32], pos: 32 };
        r.refill();
        r
    }
    fn refill(&mut self) {
        self.buf = hashv(&[&b"FDRM-M"[..], &self.seed[..], &self.block.to_le_bytes()[..]]).to_bytes();
        self.block += 1;
        self.pos = 0;
    }
    pub fn next_u32(&mut self) -> u32 {
        if self.pos + 4 > 32 {
            self.refill();
        }
        let v = u32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        v
    }
    /// Numbers for the gold drum of a particular spin
    pub fn gold(&self, serial: u32) -> (u64, u32) {
        let h = hashv(&[&b"FDRM-G"[..], &self.seed[..], &serial.to_le_bytes()[..]]).to_bytes();
        (u64::from_le_bytes(h[0..8].try_into().unwrap()), u32::from_le_bytes(h[8..12].try_into().unwrap()))
    }
}

// ---------------- win plan ----------------

/// Chances per spin in fractions of 2^32 (integers, no floating point)
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Odds {
    /// super jackpot (0 while the locks are closed)
    pub superj: u64,
    pub jackpot: u64,
    pub mini: u64,
    pub prize: u64,
    pub again: u64,
}

impl Odds {
    pub fn gold(&self) -> u64 {
        self.superj + self.jackpot + self.mini + self.prize
    }
}

const Q32: u128 = 1 << 32;

/// Chance of winning a pot on a single spin, in fractions of 2^32: the spin's contribution ÷ the pot's starting amount.
/// The starting amount is set by the administrator; the larger it is, the harder it is to win. The chance is constant while the pot
/// grows with every bet, so on average it holds about two starting amounts by the time it is won.
pub fn pot_odds(share: u64, start: u64) -> u64 {
    // share < 2^31 (the share of one bet), so share << 32 fits into u64
    ((share << 32) / start.max(1)).min(u32::MAX as u64 / 4)
}
const _: () = assert!(share(S_JACK) < (1 << 31) && share(S_MINI) < (1 << 31) && share(S_SUPER) < (1 << 31));

/// Pot starting amounts: the jackpot and the super are set by the administrator (not below the thresholds), the mini is a tenth of the jackpot
pub fn jackpot_start(init_jackpot: u64) -> u64 {
    init_jackpot.max(JP_MIN_POT)
}
pub fn mini_start(init_jackpot: u64) -> u64 {
    (init_jackpot / MINI_START_DIV).max(MINI_MIN_POT)
}
pub fn super_start(init_super: u64) -> u64 {
    init_super.max(SUPER_MIN_POT)
}
/// A pot can be won once it holds at least a quarter of the starting amount:
/// this way "pennies" cannot be won right after someone else's win if the seed reserve is empty.
pub fn open_pot(start: u64) -> u64 {
    (start as u128 * COOLDOWN_BPS as u128 / BPS as u128) as u64
}

/// Chances for one settlement: everything is computed once, only comparisons remain in the per-spin loop
/// (settling a batch of 1000 spins must fit into the transaction's compute limit).
#[derive(Debug, Clone, Copy)]
pub struct OddsBase {
    jackpot: u64,
    mini: u64,
    superj: u64,
    jackpot_min: u64,
    mini_min: u64,
    super_min: u64,
    prize: u64,
    again: u64,
}

impl OddsBase {
    /// `init_jackpot`, `init_super` — starting amounts from the config (set by the administrator)
    pub fn new_mode(init_jackpot: u64, init_super: u64, mode: u8) -> Self {
        Self::new_mode_share(init_jackpot, init_super, mode, S_SUPER)
    }
    /// The same with the super jackpot share given explicitly. The program itself always passes S_SUPER (through `new_mode`);
    /// the tests pass V1_S_SUPER to reproduce the chances of the program deployed before upgrade No. 2.
    pub fn new_mode_share(init_jackpot: u64, init_super: u64, mode: u8, s_super: u64) -> Self {
        let ppm = MODE_PPM[(mode as usize).min(MODES - 1)];
        let (j, m, s) = (jackpot_start(init_jackpot), mini_start(init_jackpot), super_start(init_super));
        OddsBase {
            jackpot: pot_odds(share(S_JACK), j),
            mini: pot_odds(share(S_MINI), m),
            superj: pot_odds(share(s_super), s),
            jackpot_min: open_pot(j),
            mini_min: open_pot(m),
            super_min: open_pot(s),
            prize: (Q32 * ppm as u128 / 1_000_000) as u64,
            again: (Q32 * W_AGAIN as u128 / BPS as u128) as u64,
        }
    }
    /// Chances at the current pots. `super_open`: the ROUND is a paid one (`!p.p_free`, the same value for every spin of the
    /// round). The free "Again" repeats of a paid round DO play the super jackpot, like every other executed spin of it; only
    /// a round placed with the contract's free spins (switched off: FREE_PER_WEEK = 0) does not.
    pub fn at_super(&self, jackpot_pool: u64, mini_pool: u64, super_pool: u64, super_open: bool) -> Odds {
        Odds {
            superj: if super_open && super_pool >= self.super_min { self.superj } else { 0 },
            jackpot: if jackpot_pool >= self.jackpot_min { self.jackpot } else { 0 },
            mini: if mini_pool >= self.mini_min { self.mini } else { 0 },
            prize: self.prize,
            again: self.again,
        }
    }
}

// ---------------- draw ----------------

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Main {
    Empty,
    Again,
    Gold,
}

/// u — a uniform 32-bit number
pub fn draw_main(u: u32, o: &Odds) -> Main {
    let r = u as u64;
    let gold = o.gold();
    if r < gold {
        Main::Gold
    } else if r < gold + o.again {
        Main::Again
    } else {
        Main::Empty
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Gold {
    Super,
    Jackpot,
    Mini,
    /// prize in whole tokens
    Prize(u64),
}

/// u1 — uniform 64-bit (jackpot or mini or prize), u2 — 32-bit (which prize). The sector shares of the gold
/// drum are proportional to the chances from `odds`. Each game mode has its own prize ladder.
pub fn draw_gold(u1: u64, u2: u32, o: &Odds) -> Gold {
    draw_gold_mode(u1, u2, o, 0)
}
pub fn draw_gold_mode(u1: u64, u2: u32, o: &Odds, mode: u8) -> Gold {
    let total = o.gold() as u128;
    let x = u1 as u128 * total; // < total × 2^64
    if x < (o.superj as u128) << 64 {
        return Gold::Super;
    }
    if x < ((o.superj + o.jackpot) as u128) << 64 {
        return Gold::Jackpot;
    }
    if x < ((o.superj + o.jackpot + o.mini) as u128) << 64 {
        return Gold::Mini;
    }
    let ladder = &MODE_LADDER[(mode as usize).min(MODES - 1)];
    let mut r = ((u2 as u64) * LADDER_SUM) >> 32;
    for (tokens, w) in ladder.iter() {
        if r < *w {
            return Gold::Prize(*tokens);
        }
        r -= *w;
    }
    Gold::Prize(ladder[0].0)
}

// ---------------- ranks, time ----------------

pub fn rank_idx(run: u64) -> usize {
    let mut idx = 0;
    for (i, r) in RANKS.iter().enumerate() {
        if run >= r.0 {
            idx = i;
        }
    }
    idx
}

pub fn streak_bonus(streak: u32) -> u16 {
    let mut b = 0;
    for (d, w) in STREAK_BONUS.iter() {
        if streak >= *d {
            b = *w;
        }
    }
    b
}

pub fn day_of(ts: i64) -> i64 {
    ts.div_euclid(DAY_SECS)
}
pub fn week_of(ts: i64) -> i64 {
    ts.div_euclid(WEEK_SECS)
}

// ---------------- round salt: hash of the bet's block ----------------

/// Hash of block `slot` from the SlotHashes system account. Data format: u64 — the number of entries, then entries
/// (u64 slot, 32-byte hash) in descending slot order. The list has no skipped slots, so we use binary search.
/// The list keeps the last 512 blocks (≈3.5 minutes): after that the hash can no longer be obtained.
pub fn slot_hash_of(data: &[u8], slot: u64) -> Option<[u8; 32]> {
    if data.len() < 8 {
        return None;
    }
    let n = (u64::from_le_bytes(data[..8].try_into().ok()?) as usize).min((data.len() - 8) / 40);
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let off = 8 + mid * 40;
        let s = u64::from_le_bytes(data[off..off + 8].try_into().ok()?);
        if s == slot {
            return data[off + 8..off + 40].try_into().ok();
        }
        if s > slot {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    None
}

/// The final round number: the house secret mixed with the hash of the bet's block. To know the outcome in advance one must know both:
/// only the house knows the secret, nobody knows the block hash at bet time.
pub fn salted(value: [u8; 32], salt: &[u8; 32]) -> [u8; 32] {
    solana_sha256_hasher::hashv(&[b"fairdrum-round-salt-v1", &value, salt]).to_bytes()
}

// ---------------- "fair draw": the house secret tree ----------------

/// Leaf hash: the secret number and the secret itself. Prefix 0 distinguishes a leaf from a node (protection against passing a node off as a leaf).
pub fn house_leaf(index: u64, secret: &[u8; 32]) -> [u8; 32] {
    solana_sha256_hasher::hashv(&[&[0u8], &index.to_le_bytes(), secret]).to_bytes()
}

/// Tree root from a leaf and the path to it. At each level a bit of the number tells whether the leaf is on the left or on the right.
pub fn house_root_of(index: u64, secret: &[u8; 32], proof: &[[u8; 32]]) -> [u8; 32] {
    let mut h = house_leaf(index, secret);
    let mut i = index;
    for sib in proof {
        h = if i & 1 == 0 { solana_sha256_hasher::hashv(&[&[1u8], &h, sib]).to_bytes() } else { solana_sha256_hasher::hashv(&[&[1u8], sib, &h]).to_bytes() };
        i >>= 1;
    }
    h
}
