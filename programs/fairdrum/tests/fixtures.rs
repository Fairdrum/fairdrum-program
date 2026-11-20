//! Writes account samples (the bytes exactly as stored on the blockchain) to check that the site reads them correctly.
//! Run together with the other tests: `cargo test -p fairdrum`. File: app/test/fixtures/accounts.json

use anchor_lang::prelude::*;
use anchor_lang::AccountSerialize;
use fairdrum::consts::*;
use fairdrum::state::*;
use std::io::Write;

fn bytes<T: AccountSerialize>(a: &T) -> Vec<u8> {
    let mut v = vec![];
    a.try_serialize(&mut v).unwrap();
    v
}
fn b64(v: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::new();
    for c in v.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}

fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{:02x}", b)).collect()
}
fn mkey(i: u64) -> Pubkey {
    let mut b = [0u8; 32];
    b[..8].copy_from_slice(&i.to_le_bytes());
    b[31] = 7;
    Pubkey::new_from_array(b)
}

#[test]
fn write_account_fixtures() {
    let owner = Pubkey::new_from_array([3; 32]);
    let mut p = Player::default();
    p.owner = owner;
    p.bump = 254;
    p.reg_no = 413;
    p.registered_at = 1_800_000_123;
    p.balance = 123_456_789_012;
    p.jp_locked = 777;
    p.jp_per_week = 88;
    p.jp_clock = 1_800_100_000;
    p.last_day = 20_833;
    p.streak = 12;
    p.run_spins = 3_456;
    p.total_spins = 9_999;
    p.act_days = 15;
    p.last_deposit_ts = 1_799_980_000;
    p.week_used = 4;
    p.pending = true;
    p.p_n = 10;
    p.p_free = false;
    p.p_commit_slot = 987_654_321;
    p.p_root = Pubkey::new_from_array([9; 32]);
    p.p_stake = 10_000_000;
    p.won_total = 424_242;

    let mut c = Config::default();
    c.bump = 253;
    c.vault_bump = 252;
    c.mint = Pubkey::new_from_array([5; 32]);
    c.vault = Pubkey::new_from_array([6; 32]);
    c.dev_authority = Pubkey::new_from_array([7; 32]);
    c.promo_authority = Pubkey::new_from_array([8; 32]);
    c.start_ts = 1_800_000_000;
    c.funded = true;
    c.registered = 1_234_567;
    c.act_pool = 25_000_000_000_000_000;
    c.prize_pool = 10_000_000_000_000;
    c.jackpot_pool = 3_914_000_000;
    c.mini_pool = 1_000_000_000;
    c.super_pool = 777_000_000_000;
    c.super_unlock_ts = 1_804_838_400;
    c.init_super = 20_000_000_000;
    c.init_jackpot = 5_000_000_000;
    c.seed_pool = 55;
    c.seed_reserve = 5_000_000_000_000_000;
    c.dev_pool = 66;
    c.promo_pool = 77;
    c.prize_buffer = 10_000_000_000_000;
    c.den = 12_000_000;
    c.epoch_start = 1_800_000_000;
    c.epoch_spins = 4_321;
    c.total_spins = 5_000_000;
    c.total_burned = 99_000_000;
    c.jackpots_won = 12;
    c.minis_won = 34;
    c.supers_won = 2;

    // PDA addresses for the fixed program: the site must derive the same ones
    let pid = Pubkey::new_from_array([11; 32]);
    let pda = |seeds: &[&[u8]]| Pubkey::find_program_address(seeds, &pid).0.to_string();
    let pdas = format!(
        "{{\"program\": \"{}\", \"owner\": \"{}\", \"config\": \"{}\", \"vault\": \"{}\", \"player\": \"{}\"}}",
        pid,
        owner,
        pda(&[b"config"]),
        pda(&[b"vault"]),
        pda(&[b"player", owner.as_ref()])
    );
    // win plan chances: the site must compute the same ones (the first number is den, then the jackpot and the mini in microtokens)
    let cases: [(u64, u64, u64, u64, bool); 7] = [
        (DEN_MIN, 5_000_000 * ONE, 500_000 * ONE, 20_000_000 * ONE, false),
        (DEN_MIN, 1, 1, 1_000_000 * ONE, true),
        (12_345, 200_000 * ONE, 20_000 * ONE, 2_000_000 * ONE, true),
        (12_345, 199_999 * ONE, 19_999 * ONE, 1_999_999 * ONE, true),
        (5_000_000, 1_500_000 * ONE, 60_000 * ONE, 20_000_000 * ONE, true),
        (5_000_000, 123_456_789 * ONE, 3_914_000 * ONE, 777_000_000 * ONE, false),
        (100, 3 * ONE, 0, 0, true),
    ];
    let live = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("consts.live.rs").exists();
    let odds_json: Vec<String> = cases
        .iter()
        .map(|(d, j, m, sp, open)| {
            // The file is the SITE's fixture, and the site describes the program deployed on mainnet. While
            // programs/fairdrum/consts.live.rs exists (the same switch as scripts/gen-economy.mjs) the chances are written with
            // the live super jackpot share (27); after the upgrade is applied and that file is removed — with this program's.
            let o = fairdrum::math::OddsBase::new_mode_share(c.init_jackpot, c.init_super, 0, if live { V1_S_SUPER } else { S_SUPER }).at_super(*j, *m, *sp, *open);
            format!(
                "{{\"den\": {}, \"jackpot\": {}, \"mini\": {}, \"super\": {}, \"superOpen\": {}, \"o\": {{\"superj\": {}, \"jackpot\": {}, \"mini\": {}, \"prize\": {}, \"again\": {}}}}}",
                d, j, m, sp, open, o.superj, o.jackpot, o.mini, o.prize, o.again
            )
        })
        .collect();
    let json = format!(
        "{{\n  \"player\": \"{}\",\n  \"config\": \"{}\",\n  \"sizes\": {{\"player\": {}, \"config\": {}}},\n  \"pdas\": {},\n  \"odds\": [{}],\n  \"minPots\": {{\"jackpot\": {}, \"mini\": {}, \"super\": {}}}\n}}\n",
        b64(&bytes(&p)),
        b64(&bytes(&c)),
        8 + Player::INIT_SPACE,
        8 + Config::INIT_SPACE,
        pdas,
        odds_json.join(","),
        fairdrum::math::open_pot(fairdrum::math::jackpot_start(c.init_jackpot)),
        fairdrum::math::open_pot(fairdrum::math::mini_start(c.init_jackpot)),
        fairdrum::math::open_pot(fairdrum::math::super_start(c.init_super))
    );
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/test/fixtures");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::File::create(dir.join("accounts.json")).unwrap().write_all(json.as_bytes()).unwrap();
    println!("sizes: player {} config {}", 8 + Player::INIT_SPACE, 8 + Config::INIT_SPACE);
}
