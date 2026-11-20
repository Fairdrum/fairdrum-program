//! The settlement of the program deployed BEFORE upgrade No. 2 (v1), for the tests only: the body of v1's `engine::settle_inner`
//! (the v1 source archive) over this crate's account types and draw primitives, with the two v1 shares (S_DEV 884, S_SUPER 27),
//! no pay caps, and the super jackpot topped up from the seed reserve. tests/xver_tests.rs proves it against the live program:
//! it reproduces the accounts the LIVE build wrote (tests/xver/after_cur.bin) byte for byte. The other tests then settle the
//! same rounds with this reference and with the program, and show exactly what the economics change changes.
#![allow(dead_code)]
use fairdrum::consts::*;
use fairdrum::engine::{self, SettleOut};
use fairdrum::math::*;
use fairdrum::state::*;

fn mul_bps(v: u64, bps: u64) -> u64 {
    ((v as u128) * (bps as u128) / (BPS as u128)) as u64
}
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
fn note_win(cfg: &mut Config, player: anchor_lang::prelude::Pubkey, kind: u8, amount: u64, ts: i64) {
    let i = (cfg.wins_head as usize) % cfg.wins.len();
    cfg.wins[i] = WinRec { player, amount, ts, kind };
    cfg.wins_head = ((i + 1) % cfg.wins.len()) as u8;
}

/// v1 settlement of a round with the round value `seed`; `late`: after the deadline (the pots pay nothing)
pub fn settle_v1(cfg: &mut Config, p: &mut Player, seed: [u8; 32], now: i64, late: bool) -> SettleOut {
    assert!(p.pending);
    let pots_open = !late;
    engine::roll_epoch_if_due(cfg, now);
    engine::apply_starts_if_due(cfg, now);
    engine::release_jackpot(p, now);
    let n = pending_n(p.p_n) as u64;
    let mode = pending_mode(p.p_n);
    let paid = !p.p_free;
    let mut out = SettleOut::default();
    let mut rng = Rng::new(seed);
    let cap = 2 * n + 16;
    let (mut jack, mut mini, mut prize, mut sd, mut reserve, mut sup) = (cfg.jackpot_pool, cfg.mini_pool, cfg.prize_pool, cfg.seed_pool, cfg.seed_reserve, cfg.super_pool);
    let (mut todo, mut exec) = (n, 0u64);
    let mut promo_extra = 0u64;
    let mut player_gain = 0u64;
    let mut awarded = 0u64;
    let ob = OddsBase::new_mode_share(p.p_start_jackpot, p.p_start_super, mode, V1_S_SUPER);
    while todo > 0 {
        todo -= 1;
        exec += 1;
        jack += share(S_JACK);
        mini += share(S_MINI);
        sup += share(V1_S_SUPER);
        sd += share(S_SEED);
        prize += share(S_PRIZE);
        let o = ob.at_super(jack, mini, sup, paid);
        match draw_main(rng.next_u32(), &o) {
            Main::Empty => {}
            Main::Again => {
                if exec < cap && prize >= COST {
                    prize -= COST;
                    todo += 1;
                    out.extras += 1;
                }
            }
            Main::Gold => {
                out.gold += 1;
                let (u1, u2) = rng.gold(exec as u32);
                match draw_gold_mode(u1, u2, &o, mode) {
                    Gold::Super | Gold::Jackpot | Gold::Mini if !pots_open => {}
                    Gold::Super => {
                        let win = mul_bps(sup, SUPER_WINNER_BPS);
                        sup -= win;
                        let top = cfg.init_super.saturating_sub(sup).min(reserve);
                        sup += top;
                        reserve -= top;
                        player_gain += grant_vested(p, win, now);
                        awarded += win;
                        note_win(cfg, p.owner, 3, win, now);
                        out.super_hits += 1;
                        out.super_total += win;
                        cfg.supers_won += 1;
                    }
                    Gold::Jackpot => {
                        let total = jack;
                        let win = mul_bps(total, JP_WINNER_BPS);
                        let to_super = mul_bps(total, JP_SUPER_BPS);
                        promo_extra += total - win - to_super;
                        sup += to_super;
                        let now_part = grant_vested(p, win, now);
                        player_gain += now_part;
                        out.jp_to_player_now += now_part;
                        awarded += win;
                        note_win(cfg, p.owner, 2, win, now);
                        out.jp_hits += 1;
                        out.jp_pool_total += total;
                        let top = cfg.init_jackpot.saturating_sub(sd).min(reserve);
                        jack = sd + top;
                        reserve -= top;
                        sd = 0;
                        cfg.jackpots_won += 1;
                    }
                    Gold::Mini => {
                        player_gain += mini;
                        awarded += mini;
                        note_win(cfg, p.owner, 1, mini, now);
                        out.mini_won += mini;
                        out.mini_hits += 1;
                        let top = mini_start(cfg.init_jackpot).min(reserve);
                        mini = top;
                        reserve -= top;
                        cfg.minis_won += 1;
                    }
                    Gold::Prize(tokens) => {
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
    cfg.jackpot_pool = jack;
    cfg.mini_pool = mini;
    cfg.prize_pool = prize;
    cfg.seed_pool = sd;
    cfg.seed_reserve = reserve;
    cfg.super_pool = sup;
    cfg.promo_pool += promo_extra;
    out.spins = exec;
    cfg.dev_pool += exec * share(V1_S_DEV);
    cfg.dev_total = cfg.dev_total.saturating_add(exec * share(V1_S_DEV));
    cfg.paid_total = cfg.paid_total.saturating_add(awarded);
    p.balance += player_gain;
    p.won_total += player_gain;
    p.run_spins += exec;
    p.total_spins += exec;
    p.pending = false;
    p.p_stake = 0;
    cfg.total_spins += exec;
    cfg.epoch_spins += exec;
    out
}
