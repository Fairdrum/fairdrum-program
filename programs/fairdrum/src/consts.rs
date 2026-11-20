//! All the numbers of the game. They change only by a rebuild and a new deploy; the contract itself has no key that edits them.
//! The site takes the figures for its pages from here as well (`npm run gen:economy` → app/src/economy.generated.ts).

// ---- Game currency: SOL (owner's decision of 11.11.2026, the "no token of our own" version) ----
// The game runs on wrapped SOL (wSOL, 9 decimals). The internal accounting unit is the "point" = 1,000 lamports
// = 0.000001 SOL: this keeps every prize table in whole numbers. A bet = 10,000 points = 0.01 SOL.
// The site shows amounts in SOL (points / 1,000,000).
pub const DECIMALS: u8 = 9;
pub const ONE: u64 = 1_000; // 1 point = 1,000 lamports = 0.000001 SOL
pub const COST: u64 = 10_000 * ONE; // bet for one spin: 0.01 SOL
/// No burning: SOL is not burned. Everything that does not go to the administrator stays in the prize bank.
pub const BURN_ENABLED: bool = false;
pub const BPS: u64 = 10_000;

// ---- Source of randomness: the "fair draw" — a house secret committed in advance, plus the hash of the bet's block ----
// There is no external oracle (see docs/RISKS.md): the round number = sha256(domain, round secret, hash of the bet's block).

// ---- Where 1 spin goes (shares in basis points, sum = 10,000) ----
// Economy of upgrade No. 2 (owner's decision of 18.11.2026: "reduce the administrator's share to 7 and send my 3% to the
// super jackpot"; the program deployed today, v1, has S_DEV = 884 and S_SUPER = 27, see consts.live.rs).
// The shares are taken from every EXECUTED spin, including the free "Again" repeats. One paid bet gives exactly
// 1 / (1 − W_AGAIN/10,000) = 1 / 0.884 executed spins on average, so in terms of PAID bets:
//   administrator  618 / 0.884 = 699.095 bps = 6.991% (v1: 884 / 0.884 = exactly 10%). 7.000% would be 618.8, and a share is a whole
//                  number of bps: 618 is the largest value that does not exceed 7% (619 would be 7.0023%);
//   super jackpot  293 / 0.884 = 3.314% of paid bets (v1: 27 → 0.305%): the 266 bps the administrator gave up are added to it;
//   prize bank, jackpot, mini, seed — unchanged, so the table prizes of a round do not change (tests/econ_tests.rs).
pub const S_PRIZE: u64 = 8_939; // prize bank: regular game prizes and "Again" repeats
pub const S_JACK: u64 = 88; // jackpot
pub const S_MINI: u64 = 44; // mini jackpot
pub const S_SUPER: u64 = 293; // super jackpot (v1: 27)
pub const S_SEED: u64 = 18; // seed of the next jackpot
pub const S_DEV: u64 = 618; // to the administrator: 6.991% of paid bets (v1: 884 = 10%; test admin_share_is_seven_percent_of_paid_volume)
pub const S_BURN: u64 = 0; // no burning (bets are in SOL)
pub const S_ACT: u64 = 0; // no activity fund
const _: () = assert!(S_PRIZE + S_JACK + S_MINI + S_SUPER + S_DEV + S_BURN + S_ACT + S_SEED == BPS);

pub const fn share(bps: u64) -> u64 {
    COST / BPS * bps // COST is a multiple of BPS, the division is exact
}
const _: () = assert!(COST % BPS == 0);

// ---- Main drum ----
/// "Again": 11.6% of spins. The rest is split between "Gold" (its chance is computed from the win plan below) and "Empty".
pub const W_AGAIN: u64 = 1_160; // out of 10,000

// ---- Jackpots: the administrator sets the starting amount, chance = contribution ÷ starting amount ----
// The starting amounts of the jackpot and the super jackpot are set by the administrator and paid from their own funds (init_jackpot,
// init_super, the seed reserve, which is topped up by the donate instruction). The chance of winning a pot on a single spin
// is constant: the spin's contribution to the pot ÷ its starting amount. The larger the starting amount, the harder it is to win:
// at 5 SOL it is 1 chance in roughly 57,000 spins. The pot grows with every bet, so by the time it is won
// it holds about two starting amounts on average: one from the administrator, the other from the players' bets.
// The seed reserve holds what was donated and, once the prize bank is above BANK_KEEP, the bank's surplus (see "The contract
// sustains itself" below). When it is empty the mini restarts from the third of the pot that stays after a win and the jackpot
// from the fifth that stays plus its seed (S_SEED); below a quarter of the starting amount a pot is closed until it has grown.
// What the administrator put in cannot be drained by series of bets: the expected win from a fresh pot per spin
// equals that spin's contribution (less than 1% of the bet).
/// Lower bound of `den` (spins over the previous week; now statistics only, it does not affect the chances)
pub const DEN_MIN: u64 = 500;
/// Smallest starting amounts (if the administrator has not set larger ones)
pub const JP_MIN_POT: u64 = 200_000 * ONE; // 0,2 SOL
pub const MINI_MIN_POT: u64 = 20_000 * ONE; // 0,02 SOL
/// Mini jackpot starting amount = jackpot starting amount ÷ MINI_START_DIV (from the seed reserve)
pub const MINI_START_DIV: u64 = 10;
/// A pot can be won once it has accumulated this share of the starting amount (protection against winning "pennies" when the reserve is empty)
pub const COOLDOWN_BPS: u64 = 2_500;
/// New starting amounts take effect this many seconds after the administrator's command (24 hours): the change is visible
/// to everyone in advance, and bets already placed are settled at the amounts in effect at the moment of the bet (audit of 11.11.2026, K1).
pub const STARTS_DELAY_SECS: i64 = 24 * 3_600;
/// Upper bound of a starting amount: 1000 SOL (audit No. 3, M3). On its own it does not protect a pot from being "switched off":
/// a pot can be won from a quarter of the starting amount, and at 1000 SOL it would stay closed until it accumulates 250 SOL
/// (audit No. 9, G2). The raise step STARTS_RAISE_MUL protects against that.
pub const START_MAX: u64 = 1_000 * 1_000_000 * ONE;
/// In one change a starting amount can be raised by at most this factor relative to the amount in effect or to the pot (whichever
/// is larger). The number is the inverse of the COOLDOWN_BPS share: a pot that holds at least the starting amount in effect
/// remains winnable after the raise. A change takes effect after 24 hours, so 0.2 → 5 SOL takes three changes.
pub const STARTS_RAISE_MUL: u64 = 4;
const _: () = assert!(STARTS_RAISE_MUL * COOLDOWN_BPS <= BPS);
/// Withdrawal lock of the program DEPLOYED today (v1): a deposit can be withdrawn 12 hours after it was made (owner's decision
/// of 11.11.2026). THIS version does NOT use this constant: upgrade No. 2 (change H) has no withdrawal lock, see
/// `DEPOSIT_LOCK_SECS` below. The line is kept, unchanged, only because the site's generator (scripts/gen-economy.mjs) and a
/// site test read the lock of the deployed program from it, and the site must keep saying "12 hours" until the upgrade is
/// applied. It does not reach the compiled program. Remove it together with the site change (docs/UPGRADE-2.md, change H).
pub const WITHDRAW_LOCK_SECS: i64 = 12 * 3_600;
/// What THIS version enforces: after a deposit the deposited amount is frozen for this many seconds. Zero by the owner's
/// decision: withdrawal is available at once. The lock protected neither the bank nor other players (no bonuses, no
/// withdrawal fee, a deposit can return only to its own wallet; audit No. 4): it was a rule, not a defence. The fields
/// `dep_locked` / `dep_unlock_ts` stay in the account layout and are still written.
pub const DEPOSIT_LOCK_SECS: i64 = 0;
/// Chance of a big prize per spin on the drum, in parts per million: 0.08% (mode 0; the rest are in MODE_PPM below)
pub const P_PRIZE_PPM: u64 = MODE_PPM[0];
/// Big prizes in tokens and their weight out of 10,000 (among prizes). There are no small prizes, the smallest is 1 million (100 bets).
pub const LADDER: [(u64, u64); 6] = MODE_LADDER[0];
pub const LADDER_SUM: u64 = 10_000;

// ---- Game modes (owner's decision of 11.11.2026) ----
// Each game has its own outcome table: the frequency and size of prizes differ, while the EXPECTED payout per spin
// is the same in every mode (and no higher than the drum's). This is the key invariant: if some game had
// a higher return, everyone would play only that one and drain the pools. It is checked by the compiler (const assert below)
// and by simulations in the tests. The pool shares, "Again", and the mini/jackpot/super are common to all modes.
/// 0 drum · 1 blackjack · 2 mines · 3 rocket · 4 plinko · 5 tower · 6 hi-lo · 7 slots 777 · 8 megaways ·
/// 9 dice · 10 coin flip · 11 roulette · 12 scratch card · 13 limbo · 14 keno · 15 diamonds
pub const MODES: usize = 16;
/// Chance of a prize per spin by mode, in parts per million (tuned for an equal expected payout)
pub const MODE_PPM: [u64; MODES] = [100_130, 329_436, 97_977, 235_562, 109_154, 78_680, 214_088, 132_819, 169_029, 267_980, 140_398, 245_330, 154_382, 223_990, 196_702, 335_500];
/// Prize tables by mode: (points, weight out of 10,000); a bet = 10,000 points. Zero weight = a spare row.
/// Economy v2: regular prizes return ≈77.5% of every spin to players (together with the "Again" repeats and
/// the jackpots — about 90% of paid bets), so prizes are frequent. The top prize of any table is ×1000 (10 SOL)
/// and contributes at most ~6% of the expected payout: a rare big prize must not decide how much a game returns,
/// otherwise with a small bank the "heavy-tailed" games would pay less than the others (audit of 11.11.2026, S2).
pub const MODE_LADDER: [[(u64, u64); 6]; MODES] = [
    // 0 drum: ×1.5 … ×1000, a prize ≈ every 12th spin
    [(15_000, 5_500), (30_000, 2_800), (100_000, 1_200), (500_000, 400), (2_500_000, 95), (10_000_000, 5)],
    // 1 blackjack: almost always ×2, a prize ≈ every 3rd round
    [(20_000, 8_700), (25_000, 1_000), (50_000, 250), (250_000, 45), (2_500_000, 5), (2_500_000, 0)],
    // 2 mines: ×3 … ×1000, a prize ≈ every 10th
    [(30_000, 6_200), (60_000, 2_500), (150_000, 950), (500_000, 300), (2_500_000, 45), (10_000_000, 5)],
    // 3 rocket: most often ×1.5, a rare tail up to ×1000; a prize ≈ every 4th
    [(15_000, 7_000), (30_000, 2_300), (100_000, 600), (500_000, 85), (2_500_000, 13), (10_000_000, 2)],
    // 4 plinko: ×2 … ×1000, a prize ≈ every 9th
    [(20_000, 6_500), (50_000, 2_500), (200_000, 800), (1_000_000, 180), (5_000_000, 17), (10_000_000, 3)],
    // 5 tower: ×4 … ×1000, a prize ≈ every 13th
    [(40_000, 6_800), (90_000, 2_300), (250_000, 700), (1_200_000, 180), (5_000_000, 17), (10_000_000, 3)],
    // 6 hi-lo: frequent ×2/×4, up to ×1000; a prize ≈ every 5th
    [(20_000, 7_000), (40_000, 2_300), (80_000, 550), (300_000, 130), (1_500_000, 18), (10_000_000, 2)],
    // 7 slots 777: ×1.5 … ×1000, a prize ≈ every 8th
    [(15_000, 5_600), (30_000, 2_900), (100_000, 1_100), (500_000, 360), (2_500_000, 37), (10_000_000, 3)],
    // 8 megaways: ×1.2 … ×1000, a prize ≈ every 6th
    [(12_000, 5_500), (25_000, 3_000), (80_000, 1_150), (400_000, 320), (2_500_000, 27), (10_000_000, 3)],
    // 9 dice: most often ×2, doubles pay up to ×360; a prize ≈ every 4th roll
    [(20_000, 7_800), (30_000, 1_500), (60_000, 500), (120_000, 160), (600_000, 35), (3_600_000, 5)],
    // 10 coin flip: a streak of heads doubles — ×2, ×4, ×8 … ×128; a prize ≈ every 7th
    [(20_000, 5_200), (40_000, 2_600), (80_000, 1_300), (160_000, 600), (320_000, 250), (1_280_000, 50)],
    // 11 roulette: colour ×2, dozen ×3, line ×6, corner ×9, split ×18, number ×36; a prize ≈ every 4th
    [(20_000, 6_000), (30_000, 2_600), (60_000, 900), (90_000, 350), (180_000, 120), (360_000, 30)],
    // 12 scratch card: bet back ×1 and prizes up to ×1000; a prize ≈ every 6th card
    [(10_000, 4_000), (20_000, 3_500), (50_000, 1_700), (200_000, 650), (1_000_000, 147), (10_000_000, 3)],
    // 13 limbo: the multiplier "flies off" to ×1.5 … ×1000; a prize ≈ every 4th–5th round
    [(15_000, 6_000), (20_000, 2_600), (50_000, 1_000), (200_000, 330), (1_000_000, 68), (10_000_000, 2)],
    // 14 keno: 3 … 8 numbers matched — from bet back ×1 up to ×1000; a prize ≈ every 5th ticket
    [(10_000, 4_500), (20_000, 3_300), (50_000, 1_500), (150_000, 600), (1_000_000, 98), (10_000_000, 2)],
    // 15 diamonds: a pair ×1.5 … five of a kind ×50; a prize ≈ every 3rd round
    [(15_000, 5_000), (20_000, 3_000), (30_000, 1_300), (50_000, 500), (100_000, 170), (500_000, 30)],
];
/// A single table prize cannot take more than this share of the prize bank: a big win does not leave the other players
/// with an empty bank. With a bank of 40 SOL or more, any prize (up to 10 SOL) is paid in full.
pub const PRIZE_BANK_DIV: u64 = 4;
/// Expected payout of a mode's prize ladder per spin, in millionths of a token
pub const fn mode_ev_ppm_tokens(m: usize) -> u128 {
    let mut s: u128 = 0;
    let mut i = 0;
    while i < 6 {
        s += MODE_LADDER[m][i].0 as u128 * MODE_LADDER[m][i].1 as u128;
        i += 1;
    }
    // s — tokens × weight (out of 10,000); EV_ppm = ppm × s / 10,000
    MODE_PPM[m] as u128 * s / 10_000
}
const fn mode_weights_ok() -> bool {
    let mut m = 0;
    while m < MODES {
        let mut s = 0u64;
        let mut i = 0;
        while i < 6 {
            s += MODE_LADDER[m][i].1;
            i += 1;
        }
        if s != LADDER_SUM {
            return false;
        }
        m += 1;
    }
    true
}
const fn mode_ev_ok() -> bool {
    let ev0 = mode_ev_ppm_tokens(0);
    let mut m = 1;
    while m < MODES {
        let ev = mode_ev_ppm_tokens(m);
        // no mode is more generous than the drum, and none is poorer than it by more than 0.5%
        if ev > ev0 || ev < ev0 - ev0 / 200 {
            return false;
        }
        m += 1;
    }
    true
}
const _: () = assert!(mode_weights_ok());
const _: () = assert!(mode_ev_ok());
// The prize bank receives S_PRIZE per spin and pays the "Again" repeats (W_AGAIN of a bet) and the table prizes out of it:
// the expected table payout cannot exceed the remainder, otherwise the bank would drain on average.
const _: () = assert!(mode_ev_ppm_tokens(0) <= (S_PRIZE - W_AGAIN) as u128 * (COST / ONE / BPS) as u128 * 1_000_000);

/// Batches larger than ×100 — only on the drum (mode 0); the other games have rounds of up to ×100.
/// Raising the limit of the other games (for example, to 1000) is the owner's decision: only this number changes, the account layout
/// is not affected (the round's bet count sits in the 12 bits of p_n, see below). What it requires and what it risks — docs/UPGRADE-2.md, section "E".
pub const ARCADE_BATCH_LIMIT: u16 = 100;
const _: () = assert!(ARCADE_BATCH_LIMIT <= BATCH_LIMIT);
pub const fn batch_limit_mode(mode: u8, registered: u64) -> u16 {
    if mode == 0 {
        batch_limit(registered)
    } else {
        ARCADE_BATCH_LIMIT
    }
}

// ---- Packing the mode into Player.p_n ----
// History: the layout of Config and Player was last changed before the mainnet launch (the SOL version of 11.11.2026). Since then
// it has not changed by a byte, and upgrade No. 2 does not change it either (docs/UPGRADE-2.md, section 2): the mode is packed
// into the existing field precisely so that no field had to be added.
// The lower 12 bits are the batch size (BATCH_LIMIT = 1000 < 4096), the upper 4 bits are the mode (MODES ≤ 16).
pub const fn pack_pending(n: u16, mode: u8) -> u16 {
    n | ((mode as u16) << 12)
}
pub const fn pending_n(p_n: u16) -> u16 {
    p_n & 0x0FFF
}
pub const fn pending_mode(p_n: u16) -> u8 {
    (p_n >> 12) as u8
}
const _: () = assert!(BATCH_LIMIT < 4_096 && MODES <= 16);

// ---- Jackpot ----
pub const JP_WINNER_BPS: u64 = 9_500; // to the winner; the promotion fund is no longer taken from the jackpot
pub const JP_SUPER_BPS: u64 = 500; // to the super jackpot
// no remainder: 95% to the winner + 5% to the super jackpot, both taken from what does not stay in the pot (JP_KEEP_BPS)
pub const JP_NOW_BPS: u64 = 2_000; // of the winner's share immediately
pub const JP_WEEKLY_BPS: u64 = 800; // of the winner's share per week, 10 weeks
pub const EPOCH_SECS: i64 = 7 * 86_400;
pub const DAY_SECS: i64 = 86_400;
pub const WEEK_SECS: i64 = 7 * 86_400;

// ---- Super jackpot ----
// The pot that accumulates for a long time. It receives S_SUPER of every spin and JP_SUPER_BPS of every jackpot paid. The chance
// per spin = S_SUPER contribution ÷ the amount set by the administrator ("starting amount" in the field names; for this pot
// it works as the TARGET SIZE: the larger it is, the rarer the win and the larger the pot when it is won). After a win the
// winner's share leaves the pot and the rest (at least 10%) stays as the beginning of the next one. Since upgrade No. 2 the
// pot is NOT topped up from the seed reserve: with 3.3% of the bets flowing in, it needs no outside money, and the reserve
// (donations and the bank's surplus) stays for the jackpot and the mini. The pot can be won from a quarter of the target
// (COOLDOWN_BPS), so after a win it accumulates for a while before it can be won again.
/// Smallest super jackpot starting amount
pub const SUPER_MIN_POT: u64 = 2_000_000 * ONE; // 2 SOL
pub const SUPER_WINNER_BPS: u64 = 9_000;
/// The super jackpot payout to the player goes in the same parts as the jackpot's (JP_NOW_BPS immediately, JP_WEEKLY_BPS per week)

// ---- No round with a positive expectation: a pot pays from a bounded amount (audit of 11.11.2026, K2; upgrade No. 2) ----
// The chance of a pot is constant (contribution ÷ starting amount) while the pot grows, so the expected win from a pot per
// spin = contribution × winner's share × (pot ÷ starting amount). A pot that has not been won for long enough would make a
// spin worth more than it costs, and with 3.3% of the bets going to the super jackpot that would happen in a noticeable
// share of its cycles. Rule: a win is computed from at most POT_CAP of the starting amounts recorded at the moment of the
// bet; whatever the pot holds above that STAYS IN THE POT for the next winner (nothing leaves the players' side).
// Then, in any state of the pots, the expected return of a spin is below the bet: checked by the compiler below.
/// A jackpot or mini win is computed from at most this many starting amounts (in bps: 30,000 = ×3)
pub const JP_PAY_CAP_BPS: u64 = 30_000;
pub const MINI_PAY_CAP_BPS: u64 = 30_000;
/// A super jackpot win is computed from at most this many starting amounts (25,000 = ×2.5)
pub const SUPER_PAY_CAP_BPS: u64 = 25_000;
/// The largest amount a pot win is computed from
pub const fn pay_cap(start: u64, cap_bps: u64) -> u64 {
    (start as u128 * cap_bps as u128 / BPS as u128) as u64
}
/// The two shares of the program deployed before upgrade No. 2 (v1; programs/fairdrum/consts.live.rs). The program does not use
/// them. They are here for the tests, which settle the same rounds with the v1 rules (tests/v1ref) and with this program and
/// show exactly what the change changes, and for the site's odds fixture while the site still describes v1.
pub const V1_S_SUPER: u64 = 27;
pub const V1_S_DEV: u64 = 884;
const _: () = assert!(S_SUPER + S_DEV == V1_S_SUPER + V1_S_DEV);
/// Upper bound of the expected pot winnings of one executed spin, in points × 10^6 × BPS² (a bet = 10,000 points):
/// Σ share × winner's share × cap. Exact: chance ≤ contribution ÷ start (rounded down), win ≤ winner's share × cap × start.
pub const fn pots_ev_bound_scaled() -> u128 {
    (S_JACK as u128 * JP_WINNER_BPS as u128 * JP_PAY_CAP_BPS as u128
        + S_MINI as u128 * BPS as u128 * MINI_PAY_CAP_BPS as u128
        + S_SUPER as u128 * SUPER_WINNER_BPS as u128 * SUPER_PAY_CAP_BPS as u128)
        * 1_000_000
}
// K2, for every state of the pots and the bank: V = value of one executed spin ≤ table prizes + pots + W_AGAIN × V (a repeat is
// one more executed spin), so V < bet  ⇐  table + pots < (1 − W_AGAIN) × bet. No mode has a larger table than the drum (mode_ev_ok).
const _: () = assert!(
    mode_ev_ppm_tokens(0) * (BPS as u128 * BPS as u128) + pots_ev_bound_scaled() < (BPS - W_AGAIN) as u128 * (COST / ONE / BPS) as u128 * 1_000_000 * (BPS as u128 * BPS as u128)
);
// the administrator can lower a starting amount to half of the pot (set_starts): the pot is then 2 starting amounts, within the caps
const _: () = assert!(JP_PAY_CAP_BPS >= 2 * BPS && MINI_PAY_CAP_BPS >= 2 * BPS && SUPER_PAY_CAP_BPS >= 2 * BPS);

// ---- The contract sustains itself: pots that restart on their own, and an outlet for the bank's surplus ----
// (owner's decision of 18.11.2026: after the program is closed nobody donates any more, and "the platform should develop
// by itself"). Two rules, both applied by the settlement itself; no instruction and no key takes part in them.
//
// 1. A jackpot or mini win leaves a share of the amount it is computed from IN THE POT, as the beginning of the next one
//    (the super jackpot has kept 10% since the economics change). The seed reserve still tops the new pot up to the
//    starting amount when it has funds; when it is empty the pot now restarts from its own share instead of from its
//    seed (jackpot) or from zero (mini). Long-run return to players is unchanged by this rule: everything a pot receives
//    is still paid to winners, only later; the average win stays about the same and the pot is larger while it waits.
// 2. The prize bank receives S_PRIZE per executed spin and pays the repeats and the table prizes, which are worth
//    slightly less (0.327% of paid bets): until this rule that difference accumulated in the bank for ever. Now, at the
//    end of every settlement, what the bank holds above BANK_KEEP moves to the seed reserve, at most BANK_RELEASE_BPS of
//    a bet per executed spin of that settlement. From the reserve it reaches players only the way it always did: as the
//    top-up of a pot after a win (never into a pot that is being played for: audit No. 3, S1).
/// Share of the amount a jackpot win is computed from that stays in the pot (20%). Of the rest the winner receives
/// JP_WINNER_BPS (95%) and the super jackpot JP_SUPER_BPS (5%): 76% and 4% of that amount.
pub const JP_KEEP_BPS: u64 = 2_000;
/// Share of the amount a mini win is computed from that stays in the pot (a third); the winner receives the rest.
pub const MINI_KEEP_BPS: u64 = 3_333;
const _: () = assert!(JP_KEEP_BPS > 0 && MINI_KEEP_BPS > 0 && JP_KEEP_BPS <= BPS / 2 && MINI_KEEP_BPS <= BPS / 2);
/// The prize bank keeps this much; only what it holds above is released to the seed reserve. 1,600 SOL = four largest
/// table prizes (so that any prize is paid in full) + seven mean depths of the bank's random walk below the threshold
/// (checked for every game below).
pub const BANK_KEEP: u64 = 1_600 * 1_000_000 * ONE;
/// Upper limit of the release per executed spin of the settlement, in bps of a bet: 0.5% (50,000 lamports).
pub const BANK_RELEASE_BPS: u64 = 50;
/// How much the bank releases to the seed reserve at the end of a settlement that executed `executed` spins and left the
/// bank at `prize_pool`. A pure function of these two numbers: nobody chooses the amount or the moment.
pub const fn bank_release(prize_pool: u64, executed: u64) -> u64 {
    let excess = prize_pool.saturating_sub(BANK_KEEP);
    let limit = executed.saturating_mul(share(BANK_RELEASE_BPS));
    if excess < limit {
        excess
    } else {
        limit
    }
}
/// What the bank gains per executed spin of game `m` on average, in points × 10^6: its share minus repeats minus table prizes.
pub const fn bank_surplus_ppm(m: usize) -> u128 {
    (S_PRIZE - W_AGAIN) as u128 * (COST / ONE / BPS) as u128 * 1_000_000 - mode_ev_ppm_tokens(m)
}
/// Second moment of what the bank pays for one executed spin of game `m` (a table prize or a repeat), points² × 10^6.
/// It is an upper bound of the variance of the bank's change per spin.
pub const fn bank_out_sq_ppm(m: usize) -> u128 {
    let mut s: u128 = 0;
    let mut i = 0;
    while i < 6 {
        s += MODE_LADDER[m][i].0 as u128 * MODE_LADDER[m][i].0 as u128 * MODE_LADDER[m][i].1 as u128;
        i += 1;
    }
    MODE_PPM[m] as u128 * s / 10_000 + W_AGAIN as u128 * 100 * (COST / ONE) as u128 * (COST / ONE) as u128
}
/// The largest table prize of any game, in points
pub const fn max_table_prize() -> u64 {
    let (mut m, mut best) = (0, 0u64);
    while m < MODES {
        let mut i = 0;
        while i < 6 {
            if MODE_LADDER[m][i].1 > 0 && MODE_LADDER[m][i].0 > best {
                best = MODE_LADDER[m][i].0;
            }
            i += 1;
        }
        m += 1;
    }
    best
}
/// The buffer above "any prize is paid in full" is at least this many mean depths (variance ÷ 2 × drift) of the bank below
/// the threshold: the share of time the bank spends that deep is below e^-7 ≈ 0.09%.
pub const BANK_BUFFER_DEPTHS: u128 = 7;
const fn bank_rules_ok() -> bool {
    let buffer = (BANK_KEEP / ONE - PRIZE_BANK_DIV * max_table_prize()) as u128;
    let limit = share(BANK_RELEASE_BPS) as u128 / ONE as u128 * 1_000_000; // points × 10^6 per executed spin
    let mut m = 0;
    while m < MODES {
        let gain = bank_surplus_ppm(m);
        // the buffer: at least BANK_BUFFER_DEPTHS mean depths in every game
        if buffer * 2 * gain < BANK_BUFFER_DEPTHS * bank_out_sq_ppm(m) {
            return false;
        }
        // the release outruns the bank's gain by half at least (the bank cannot grow without limit above the threshold),
        // and is at most 1% of a bet per spin (an excess that exists on the day of the upgrade leaves gradually)
        if limit * 2 < gain * 3 {
            return false;
        }
        m += 1;
    }
    BANK_RELEASE_BPS <= 100 && BANK_KEEP / ONE > PRIZE_BANK_DIV * max_table_prize()
}
const _: () = assert!(bank_rules_ok());
/// What the pots ask of the seed reserve per executed spin when every restart is topped up to the starting amount, in
/// points × 10^6 (model: a pot restarted at one starting amount is won after an exponential number of spins with the mean
/// starting amount ÷ contribution, so it holds 1 + Exp(1) starting amounts; e^-2 of a starting amount lies above the ×3
/// pay cap on average and stays; the jackpot also has its seed). The reserve receives at most the bank's gain. The demand
/// must exceed it, otherwise the released surplus would pile up in the reserve instead of in the bank.
/// This figure (35.97 points) is a LOWER estimate, not the demand itself: the model is linear, so a restart that already
/// holds more than one starting amount (no top-up, and nothing goes back to the reserve) is netted against the others,
/// while a top-up is never negative. Measured by the re-verification of 18.11.2026 with every restart topped up: about
/// 49.8 points per executed spin (35.4 jackpot + 14.4 mini), 1.72 × the bank's gain, where this model gives 1.24 ×.
/// The assert below compares the lower estimate with the gain, so it holds for the true demand with room to spare.
/// (While a large v1-era bank is being released at the limit of 50 points, supply about equals demand and the reserve
/// does not drain for that period; it does once the bank is back at BANK_KEEP.)
pub const fn reserve_demand_ppm() -> u128 {
    const E2: u128 = 135_335; // e^-2 × 10^6
    let jack = 1_000_000 - S_SEED as u128 * 1_000_000 / S_JACK as u128 - E2 - (2_000_000 - E2) * JP_KEEP_BPS as u128 / BPS as u128;
    let mini = 1_000_000 - E2 - (2_000_000 - E2) * MINI_KEEP_BPS as u128 / BPS as u128;
    S_JACK as u128 * jack + S_MINI as u128 * mini
}
const fn reserve_is_used() -> bool {
    let mut m = 0;
    while m < MODES {
        if reserve_demand_ppm() * 10 < bank_surplus_ppm(m) * 12 {
            return false;
        }
        m += 1;
    }
    // the pay caps the model assumes
    JP_PAY_CAP_BPS == 30_000 && MINI_PAY_CAP_BPS == 30_000
}
const _: () = assert!(reserve_is_used());
/// The bound of `pots_ev_bound_scaled` with the kept shares taken into account, in points × 10^6 × BPS³. A win is at most
/// (1 − kept share) × winner's share × cap, plus one lamport of rounding (added below as a whole point per pot).
pub const fn pots_ev_kept_bound_scaled() -> u128 {
    (S_JACK as u128 * JP_WINNER_BPS as u128 * (BPS - JP_KEEP_BPS) as u128 * JP_PAY_CAP_BPS as u128
        + S_MINI as u128 * BPS as u128 * (BPS - MINI_KEEP_BPS) as u128 * MINI_PAY_CAP_BPS as u128
        + S_SUPER as u128 * SUPER_WINNER_BPS as u128 * BPS as u128 * SUPER_PAY_CAP_BPS as u128)
        * 1_000_000
}
// The kept shares only lower what a spin can win: the bound of K2 above holds unchanged, and the tighter one too.
const _: () = assert!(pots_ev_kept_bound_scaled() <= pots_ev_bound_scaled() * BPS as u128);
const _: () = assert!(
    mode_ev_ppm_tokens(0) * (BPS as u128 * BPS as u128 * BPS as u128) + pots_ev_kept_bound_scaled() + 2 * 1_000_000 * (BPS as u128 * BPS as u128 * BPS as u128)
        < (BPS - W_AGAIN) as u128 * (COST / ONE / BPS) as u128 * 1_000_000 * (BPS as u128 * BPS as u128 * BPS as u128)
);

// ---- Activity ----
/// Free spins per week for each player (0 disables free spins)
pub const FREE_PER_WEEK: u16 = 0; // no free spins: in SOL that is a handout of value that bots would drain
pub const PENDING_EXPIRY_SLOTS: u64 = 3_000; // ≈20 minutes

/// Ranks: (spin threshold in the streak, withdrawal fee in bps)
pub const RANKS: [(u64, u64); 5] = [(0, 0), (100, 0), (1_000, 0), (10_000, 0), (100_000, 0)]; // withdrawal without a fee
/// Streak bonus: (days in a row, free spins per week)
pub const STREAK_BONUS: [(u32, u16); 3] = [(7, 0), (30, 0), (90, 0)];

/// Maximum batch size. It used to grow in steps with the number of players (10 → 1000); after live measurements
/// on devnet (settling ×1000 ≈ 0.11 million CU out of the 1.4 million limit — a tenfold margin) the steps were removed by
/// the owner's decision of 10.11.2026: batches of up to 1000 are open to everyone from day one. The upper bound remains —
/// it protects the transaction's compute limit and was verified by a storm of batches of 1000.
pub const BATCH_LIMIT: u16 = 1_000;
/// Smallest round size: 10 bets = 0.1 SOL (the same rule as on the site; audit No. 8, F2).
/// A one-bet round costs the keeper almost as much as a ×100 one, and a queue of such rounds is the cheapest way
/// to overload settlement. It does not affect bets already placed: settlement and refund do not check the round size.
pub const MIN_BATCH: u16 = 10;
const _: () = assert!(MIN_BATCH >= 1 && MIN_BATCH <= ARCADE_BATCH_LIMIT && MIN_BATCH <= SESSION_MAX_BATCH && SESSION_MAX_BATCH <= BATCH_LIMIT);
pub const fn batch_limit(_registered: u64) -> u16 {
    BATCH_LIMIT
}

// ---- PDA seeds ----
pub const SEED_CONFIG: &[u8] = b"config";
pub const SEED_VAULT: &[u8] = b"vault";
pub const SEED_PLAYER: &[u8] = b"player";
/// Seed of the game session PDA (session key: spins without a wallet signature)
pub const SEED_SESSION: &[u8] = b"sess";
/// Maximum session lifetime, seconds (72 hours)
pub const SESSION_MAX_SECS: i64 = 72 * 3600;
/// The largest round a session key can place (1 SOL); the owner's wallet — up to BATCH_LIMIT.
/// A stolen session key cannot burn the whole session limit with one ×1000 transaction (audit No. 8, I-1).
pub const SESSION_MAX_BATCH: u16 = 100;

/// Maximum depth of the house secret tree (2^21 ≈ 2 million bets per root; more does not fit into one settlement transaction)
pub const HOUSE_MAX_DEPTH: u8 = 21;
/// The hash of the bet's block is available to the contract for 512 blocks; with a margin for skipped blocks, an unsealed round is closed later
pub const SEAL_WINDOW_SLOTS: u64 = 600;
/// If the house has not revealed the secret of a sealed round within this time (≈ 24 hours), the bet is returned to the player
pub const REVEAL_TIMEOUT_SLOTS: u64 = 216_000;

// ---- personal deposit address ----
pub const SEED_DEPOSIT: &[u8] = b"deposit";
// ---- withdrawal in regular SOL (the withdraw_sol command) ----
/// Service account of wrapped SOL through which a withdrawal is turned into regular SOL: the address is derived from the program, the owner
/// of the account is the config account. Between commands it is empty (it holds only the network's rent deposit for the record). Created once by the init_unwrap command (administrator only).
pub const SEED_UNWRAP: &[u8] = b"unwrap";
/// Record of the root authority (the dedicated key that sets the secrets root): one per program
pub const SEED_ROOT_AUTH: &[u8] = b"root-auth";
/// Size of an SPL Token token account in bytes
pub const TOKEN_ACCOUNT_LEN: u64 = 165;
/// Smallest amount the deposit_sweep command credits; anything less waits at the address until enough accumulates
pub const SWEEP_MIN: u64 = 10_000_000; // 0,01 SOL
/// Reimbursement of the network fee to whoever sent the crediting command (usually the keeper)
pub const SWEEP_FEE_REFUND: u64 = 30_000;
/// If the player's wallet holds less than this, part of the deposit goes to their wallet — for network fees on bets and withdrawals
pub const WALLET_FLOOR: u64 = 3_000_000;
pub const WALLET_DRIP: u64 = 5_000_000;
