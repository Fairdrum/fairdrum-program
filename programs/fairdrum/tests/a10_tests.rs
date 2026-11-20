//! Who may set the root of the house secrets, and the dedicated ROOT AUTHORITY key (audit No. 10, P10-1; closing the program,
//! owner's option A). Every case goes through the program's real entry point (`fairdrum::entry`: instruction dispatch, Anchor's
//! account validation, the instruction body) on hand-built accounts: the config account, the program data account as the
//! upgradeable loader stores it, the `RootAuth` record. No Solana node is involved, so the one thing not run here is the
//! CREATION of the `RootAuth` account (a system-program call); that is run on a local validator (tests/local/run-close.sh).
//! Run: `cargo test -p fairdrum --release --offline --test a10_tests -- --nocapture`

use anchor_lang::prelude::*;
use anchor_lang::solana_program::bpf_loader_upgradeable;
use anchor_lang::InstructionData;
use anchor_spl::token::spl_token;
use fairdrum::consts::{SEED_CONFIG, SEED_ROOT_AUTH, SEED_UNWRAP, SEED_VAULT};
use fairdrum::errors::FdError;
use fairdrum::state::{Config, RootAuth};
use std::collections::BTreeSet;

type R = std::result::Result<(), ProgramError>;
const SYSTEM: Pubkey = anchor_lang::system_program::ID;
const LOADER: Pubkey = bpf_loader_upgradeable::ID;

/// The program data account as the upgradeable loader writes it: variant 3, slot, optional upgrade authority
fn program_data_bytes(authority: Option<Pubkey>) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&3u32.to_le_bytes());
    d.extend_from_slice(&7u64.to_le_bytes());
    match authority {
        Some(k) => {
            d.push(1);
            d.extend_from_slice(k.as_ref());
        }
        None => {
            d.push(0);
            d.extend_from_slice(&[0u8; 32]);
        }
    }
    d.extend_from_slice(&[0u8; 64]); // the program code would follow
    d
}

#[derive(Clone)]
struct Acc {
    key: Pubkey,
    signer: bool,
    writable: bool,
    lamports: u64,
    data: Vec<u8>,
    owner: Pubkey,
    executable: bool,
}
fn acc(key: Pubkey, owner: Pubkey, data: Vec<u8>) -> Acc {
    Acc { key, signer: false, writable: false, lamports: 10_000_000, data, owner, executable: false }
}
fn signer(key: Pubkey) -> Acc {
    Acc { signer: true, writable: true, ..acc(key, SYSTEM, vec![]) }
}
fn w(mut a: Acc) -> Acc {
    a.writable = true;
    a
}

/// Off a node the program has no rent and clock values: give it fixed ones (they do not take part in any permission check)
struct Stubs;
impl anchor_lang::compat::solana_sysvar::program_stubs::SyscallStubs for Stubs {
    fn sol_get_rent_sysvar(&self, var_addr: *mut u8) -> u64 {
        unsafe { *(var_addr as *mut Rent) = Rent::default() };
        0
    }
    fn sol_get_clock_sysvar(&self, var_addr: *mut u8) -> u64 {
        unsafe { *(var_addr as *mut Clock) = Clock { unix_timestamp: 1_800_000_000, slot: 1_000, ..Clock::default() } };
        0
    }
}
static STUBS: std::sync::Once = std::sync::Once::new();

/// Runs one instruction through the program entry point; on success the account data is written back (as the runtime would)
fn run(accs: &mut [Acc], data: Vec<u8>) -> R {
    STUBS.call_once(|| {
        anchor_lang::compat::solana_sysvar::program_stubs::set_syscall_stubs(Box::new(Stubs));
    });
    let mut copies: Vec<Acc> = accs.to_vec();
    let res = {
        let infos: Vec<AccountInfo> =
            copies.iter_mut().map(|a| AccountInfo::new(&a.key, a.signer, a.writable, &mut a.lamports, &mut a.data[..], &a.owner, a.executable)).collect();
        fairdrum::entry(&fairdrum::ID, &infos, &data)
    };
    if res.is_ok() {
        accs.clone_from_slice(&copies);
    }
    res
}
fn is(r: R, e: FdError) -> bool {
    matches!(r, Err(ProgramError::Custom(c)) if c == u32::from(e))
}
fn is_anchor(r: R, e: anchor_lang::error::ErrorCode) -> bool {
    matches!(r, Err(ProgramError::Custom(c)) if c == u32::from(e))
}

/// The world of one test: the keys of every role and the three program accounts
struct World {
    admin: Pubkey,
    promo: Pubkey,
    guardian: Pubkey,
    upgrade: Pubkey,
    root_key: Pubkey,
    config: Acc,
    program_data: Acc,
    root_auth: Acc,
}
fn config_key() -> Pubkey {
    Pubkey::find_program_address(&[SEED_CONFIG], &fairdrum::ID).0
}
fn root_auth_key() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SEED_ROOT_AUTH], &fairdrum::ID)
}
fn root_auth_bytes(authority: Pubkey, pending: Pubkey) -> Vec<u8> {
    let mut d = Vec::new();
    RootAuth { authority, pending, bump: root_auth_key().1 }.try_serialize(&mut d).unwrap();
    assert_eq!(d.len(), 8 + RootAuth::INIT_SPACE);
    d
}
/// `with_root_key`: the `RootAuth` record exists and names `root_key`; otherwise the address is an empty system account
fn world(with_root_key: bool, recorded_upgrade_authority: bool) -> World {
    let (admin, promo, guardian, upgrade, root_key) = (Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique(), Pubkey::new_unique());
    let mut cfg = Config::default();
    cfg.bump = Pubkey::find_program_address(&[SEED_CONFIG], &fairdrum::ID).1;
    cfg.vault_bump = Pubkey::find_program_address(&[SEED_VAULT], &fairdrum::ID).1;
    cfg.vault = Pubkey::find_program_address(&[SEED_VAULT], &fairdrum::ID).0;
    cfg.mint = spl_token::native_mint::ID;
    cfg.dev_authority = admin;
    cfg.promo_authority = promo;
    cfg.guardian = guardian;
    cfg.funded = true;
    cfg.dev_pool = 5_000_000_000;
    cfg.promo_pool = 5_000_000_000;
    cfg.seed_reserve = 5_000_000_000;
    cfg.init_jackpot = 200_000_000;
    cfg.init_super = 2_000_000_000;
    let mut cfg_data = Vec::new();
    cfg.try_serialize(&mut cfg_data).unwrap();
    cfg_data.resize(8 + Config::INIT_SPACE, 0);
    let root_auth = if with_root_key { acc(root_auth_key().0, fairdrum::ID, root_auth_bytes(root_key, Pubkey::default())) } else { Acc { lamports: 0, ..acc(root_auth_key().0, SYSTEM, vec![]) } };
    World {
        admin,
        promo,
        guardian,
        upgrade,
        root_key,
        config: acc(config_key(), fairdrum::ID, cfg_data),
        program_data: acc(fairdrum::program_data_address(&fairdrum::ID), LOADER, program_data_bytes(if recorded_upgrade_authority { Some(upgrade) } else { None })),
        root_auth,
    }
}
impl World {
    fn cfg(&self) -> Config {
        Config::try_deserialize(&mut &self.config.data[..]).unwrap()
    }
    fn ra(&self) -> RootAuth {
        RootAuth::try_deserialize(&mut &self.root_auth.data[..]).unwrap()
    }
    /// set_house_root signed by `who` with the world's accounts; a fresh random root of depth 3
    fn set_root(&mut self, who: Pubkey) -> R {
        let (pd, ra) = (self.program_data.clone(), self.root_auth.clone());
        self.set_root_with(signer(who), pd, ra)
    }
    fn set_root_with(&mut self, who: Acc, program_data: Acc, root_auth: Acc) -> R {
        let root = Pubkey::new_unique().to_bytes();
        let before = self.cfg();
        let mut a = [who, program_data, w(self.config.clone()), root_auth];
        let r = run(&mut a, fairdrum::instruction::SetHouseRoot { root, depth: 3 }.data());
        if r.is_ok() {
            self.config = a[2].clone();
            assert_eq!((self.cfg().house_root, self.cfg().house_depth, self.cfg().house_start), (root, 3, before.round_counter), "the accepted command wrote the new root");
        } else {
            assert_eq!(self.cfg().house_root, before.house_root, "a rejected command changes nothing");
        }
        r
    }
    fn set_root_authority(&mut self, who: Pubkey, key: Pubkey) -> R {
        let mut a = [signer(who), self.program_data.clone(), self.config.clone(), w(self.root_auth.clone()), Acc { executable: true, ..acc(SYSTEM, Pubkey::default(), vec![]) }];
        let r = run(&mut a, fairdrum::instruction::SetRootAuthority { key }.data());
        if r.is_ok() {
            self.root_auth = a[3].clone();
        }
        r
    }
    fn propose(&mut self, who: Pubkey, new_authority: Pubkey) -> R {
        let mut a = [signer(who), w(self.root_auth.clone())];
        let r = run(&mut a, fairdrum::instruction::ProposeRootAuthority { new_authority }.data());
        if r.is_ok() {
            self.root_auth = a[1].clone();
        }
        r
    }
    fn accept(&mut self, who: Pubkey) -> R {
        let mut a = [signer(who), self.config.clone(), w(self.root_auth.clone())];
        let r = run(&mut a, fairdrum::instruction::AcceptRootAuthority {}.data());
        if r.is_ok() {
            self.root_auth = a[2].clone();
        }
        r
    }
}

/// No `RootAuth` record: the v2 rule of audit No. 10. Only the holder of the upgrade authority sets the root.
#[test]
fn a10_without_a_root_authority_only_the_upgrade_authority_sets_the_root() {
    let mut t = world(false, true);
    let (admin, upgrade) = (t.admin, t.upgrade);
    t.set_root(upgrade).expect("the upgrade authority sets the root");
    // the administrator key alone is rejected (the stolen-admin-key scenario of audits No. 8 and No. 10); so are the other roles
    assert!(is(t.set_root(admin), FdError::Unauthorized), "the administrator key alone must be rejected");
    for k in [t.promo, t.guardian, t.root_key, Pubkey::new_unique()] {
        assert!(is(t.set_root(k), FdError::Unauthorized));
    }
    // if one key holds both roles, it is accepted as the upgrade authority (not because it is the administrator)
    t.program_data.data = program_data_bytes(Some(admin));
    t.set_root(admin).expect("one key in both roles");
    t.program_data.data = program_data_bytes(Some(upgrade));

    let (pd, ra) = (t.program_data.clone(), t.root_auth.clone());
    // a program data account of ANOTHER program whose authority is the caller
    let foreign = Acc { key: fairdrum::program_data_address(&Pubkey::new_unique()), data: program_data_bytes(Some(admin)), ..pd.clone() };
    assert!(is(t.set_root_with(signer(admin), foreign, ra.clone()), FdError::Unauthorized));
    // the right address, but not owned by the upgradeable loader
    assert!(is(t.set_root_with(signer(upgrade), Acc { owner: fairdrum::ID, ..pd.clone() }, ra.clone()), FdError::Unauthorized));
    // the right address and owner, unreadable or of another loader state (variant 2 = a program account, not program data)
    assert!(is(t.set_root_with(signer(upgrade), Acc { data: vec![], ..pd.clone() }, ra.clone()), FdError::Unauthorized));
    let mut other_state = program_data_bytes(Some(upgrade));
    other_state[0] = 2;
    assert!(is(t.set_root_with(signer(upgrade), Acc { data: other_state, ..pd.clone() }, ra.clone()), FdError::Unauthorized));
    // the upgrade authority named but not signing
    assert!(t.set_root_with(Acc { signer: false, ..signer(upgrade) }, pd.clone(), ra.clone()).is_err());
    // another account in place of the `RootAuth` address (the caller cannot choose which record is consulted)
    let elsewhere = Acc { key: Pubkey::new_unique(), ..ra.clone() };
    assert!(is_anchor(t.set_root_with(signer(upgrade), pd.clone(), elsewhere), anchor_lang::error::ErrorCode::ConstraintSeeds));
    // something at the `RootAuth` address that this program does not own (cannot be created on the network; here: any content)
    // is "no record": the upgrade-authority rule applies, and the key written in it means nothing
    let stranger = Pubkey::new_unique();
    let fake = Acc { owner: Pubkey::new_unique(), data: root_auth_bytes(stranger, Pubkey::default()), ..ra.clone() };
    assert!(is(t.set_root_with(signer(stranger), pd.clone(), fake.clone()), FdError::Unauthorized));
    t.set_root_with(signer(upgrade), pd.clone(), fake).expect("still the upgrade authority");
    // lamports sent to the empty address by an outsider change nothing
    t.set_root_with(signer(upgrade), pd.clone(), Acc { lamports: 5_000_000, ..ra.clone() }).expect("a funded but empty address is still no record");
}

/// The forms of the instruction used before (administrator, config) and (authority, program data, config) no longer pass.
#[test]
fn a10_older_account_forms_are_rejected() {
    let t = world(false, true);
    let data = fairdrum::instruction::SetHouseRoot { root: [7u8; 32], depth: 3 }.data();
    let mut two = [signer(t.admin), w(t.config.clone())];
    assert!(run(&mut two, data.clone()).is_err());
    let mut three = [signer(t.upgrade), t.program_data.clone(), w(t.config.clone())];
    assert!(is_anchor(run(&mut three, data), anchor_lang::error::ErrorCode::AccountNotEnoughKeys));
}

/// The upgrade authority was revoked and no root authority had been named: nobody can set a root, and nothing panics.
#[test]
fn close_revoked_without_a_root_authority_nobody_sets_the_root() {
    let mut t = world(false, false);
    for k in [t.admin, t.upgrade, t.root_key, t.promo, t.guardian, Pubkey::default()] {
        assert!(is(t.set_root(k), FdError::Unauthorized));
        // (off a node the record cannot be created, so the exact error is checked where the record exists:
        // close_set_root_authority_checks; on a node: tests/local/run-close.sh)
        assert!(t.set_root_authority(k, Pubkey::new_unique()).is_err(), "and a root authority can no longer be named");
        assert_eq!(t.root_auth.owner, SYSTEM);
    }
}

/// The `RootAuth` record exists: exactly one key sets the root, with or without an upgrade authority.
#[test]
fn close_with_a_root_authority_only_that_key_sets_the_root() {
    for upgrade_exists in [true, false] {
        let mut t = world(true, upgrade_exists);
        let (root_key, upgrade, admin) = (t.root_key, t.upgrade, t.admin);
        t.set_root(root_key).expect("the root authority sets the root");
        // the upgrade authority is refused on this path: there is exactly one party that can change the root
        assert!(is(t.set_root(upgrade), FdError::NotRootAuthority), "the upgrade authority must be refused once a root authority exists");
        for k in [admin, t.promo, t.guardian, Pubkey::new_unique(), Pubkey::default()] {
            assert!(is(t.set_root(k), FdError::NotRootAuthority));
        }
        // the root authority named but not signing
        let (pd, ra) = (t.program_data.clone(), t.root_auth.clone());
        assert!(t.set_root_with(Acc { signer: false, ..signer(root_key) }, pd.clone(), ra.clone()).is_err());
        // the program data account is not consulted at all on this path: unreadable, foreign, another owner — the root key still works
        // (so nothing depends on the loader after the program is closed)
        t.set_root_with(signer(root_key), Acc { data: vec![], ..pd.clone() }, ra.clone()).expect("program data unreadable");
        t.set_root_with(signer(root_key), Acc { key: Pubkey::new_unique(), owner: SYSTEM, data: vec![], ..pd.clone() }, ra.clone()).expect("any account in the program data slot");
        // a record of this program's type at ANOTHER address naming the attacker
        let thief = Pubkey::new_unique();
        let forged = Acc { key: Pubkey::new_unique(), data: root_auth_bytes(thief, Pubkey::default()), ..ra.clone() };
        assert!(is_anchor(t.set_root_with(signer(thief), pd.clone(), forged), anchor_lang::error::ErrorCode::ConstraintSeeds));
        // a damaged record owned by the program is an error for everyone; it never falls back to the upgrade authority
        let mut broken = ra.clone();
        broken.data[0] ^= 0xff;
        assert!(t.set_root_with(signer(root_key), pd.clone(), broken.clone()).is_err());
        assert!(t.set_root_with(signer(upgrade), pd.clone(), broken).is_err());
        // a record with an empty key (the instructions never write one) accepts nobody
        let empty = Acc { data: root_auth_bytes(Pubkey::default(), Pubkey::default()), ..ra.clone() };
        assert!(is(t.set_root_with(signer(Pubkey::default()), pd.clone(), empty), FdError::NotRootAuthority));
        // the rules of a root change are the usual ones (engine::set_root): a root already used is refused
        let used = t.cfg().house_prev[0];
        let mut a = [signer(root_key), pd.clone(), w(t.config.clone()), ra.clone()];
        assert!(is(run(&mut a, fairdrum::instruction::SetHouseRoot { root: used, depth: 3 }.data()), FdError::BadParams));
        assert!(is(run(&mut a, fairdrum::instruction::SetHouseRoot { root: [9u8; 32], depth: 0 }.data()), FdError::BadParams));
        assert!(is(run(&mut a, fairdrum::instruction::SetHouseRoot { root: [9u8; 32], depth: fairdrum::consts::HOUSE_MAX_DEPTH + 1 }.data()), FdError::BadParams));
    }
}

/// `set_root_authority`: only the holder of the upgrade authority, only while one exists; the key must have no other role.
#[test]
fn close_set_root_authority_checks() {
    let mut t = world(true, true);
    let (upgrade, admin, old_root) = (t.upgrade, t.admin, t.root_key);
    let new_root = Pubkey::new_unique();
    // a hand-over is in progress; the replacement cancels it
    t.propose(old_root, Pubkey::new_unique()).unwrap();
    for k in [admin, old_root, t.promo, t.guardian, Pubkey::new_unique()] {
        assert!(is(t.set_root_authority(k, new_root), FdError::Unauthorized), "only the upgrade authority names the root authority");
    }
    // keys that already have a role, and the empty key, are refused
    let pending_admin = Pubkey::new_unique();
    let mut cfg = t.cfg();
    cfg.pending_dev = pending_admin;
    let mut d = Vec::new();
    cfg.try_serialize(&mut d).unwrap();
    d.resize(8 + Config::INIT_SPACE, 0);
    t.config.data = d;
    for k in [Pubkey::default(), admin, pending_admin, t.promo, t.guardian] {
        assert!(is(t.set_root_authority(upgrade, k), FdError::BadParams));
    }
    assert_eq!(t.ra().authority, old_root, "rejected commands changed nothing");
    t.set_root_authority(upgrade, new_root).expect("the upgrade authority replaces the root authority");
    assert_eq!((t.ra().authority, t.ra().pending, t.ra().bump), (new_root, Pubkey::default(), root_auth_key().1));
    assert!(is(t.set_root(old_root), FdError::NotRootAuthority), "the replaced key is refused");
    t.set_root(new_root).expect("the new key sets the root");
    // the upgrade authority may also be the root authority (e.g. a multisig holding both before closing)
    t.set_root_authority(upgrade, upgrade).unwrap();
    t.set_root(upgrade).unwrap();
    t.set_root_authority(upgrade, new_root).unwrap();
    // wrong accounts: a foreign program data account, a record at another address
    let mut a = [signer(admin), Acc { key: fairdrum::program_data_address(&Pubkey::new_unique()), data: program_data_bytes(Some(admin)), ..t.program_data.clone() }, t.config.clone(), w(t.root_auth.clone()), Acc { executable: true, ..acc(SYSTEM, Pubkey::default(), vec![]) }];
    assert!(is(run(&mut a, fairdrum::instruction::SetRootAuthority { key: Pubkey::new_unique() }.data()), FdError::Unauthorized));
    let mut a = [signer(upgrade), t.program_data.clone(), t.config.clone(), w(Acc { key: Pubkey::new_unique(), ..t.root_auth.clone() }), Acc { executable: true, ..acc(SYSTEM, Pubkey::default(), vec![]) }];
    assert!(is_anchor(run(&mut a, fairdrum::instruction::SetRootAuthority { key: Pubkey::new_unique() }.data()), anchor_lang::error::ErrorCode::ConstraintSeeds));

    // the upgrade authority is revoked: the instruction is closed for everyone, the former authority included
    t.program_data.data = program_data_bytes(None);
    for k in [upgrade, admin, new_root, Pubkey::default()] {
        assert!(is(t.set_root_authority(k, Pubkey::new_unique()), FdError::Unauthorized));
    }
    assert_eq!(t.ra().authority, new_root);
    t.set_root(new_root).expect("the root authority keeps working after the revocation");
}

/// Hand-over by the root authority itself: two steps, wrong signers, replays, cancellation.
#[test]
fn close_hand_over_of_the_root_authority() {
    let mut t = world(true, false); // the program is already closed
    let (old, admin, upgrade) = (t.root_key, t.admin, t.upgrade);
    let (new, typo) = (Pubkey::new_unique(), Pubkey::new_unique());
    // nothing is proposed: nobody can accept (the empty key included)
    for k in [new, old, admin, Pubkey::default()] {
        assert!(is(t.accept(k), FdError::NotRootAuthority));
    }
    // only the root authority proposes
    for k in [admin, upgrade, t.promo, t.guardian, new, Pubkey::default()] {
        assert!(is(t.propose(k, new), FdError::NotRootAuthority));
    }
    // a typo: a key nobody holds is proposed. Nothing is lost: the old key stays in charge and proposes again
    t.propose(old, typo).unwrap();
    assert_eq!((t.ra().authority, t.ra().pending), (old, typo));
    t.set_root(old).expect("until the new key accepts, the old key is the root authority");
    assert!(is(t.set_root(typo), FdError::NotRootAuthority), "a proposed key has no right yet");
    assert!(is(t.accept(new), FdError::NotRootAuthority));
    t.propose(old, new).unwrap();
    t.propose(old, new).expect("repeating the proposal is harmless");
    assert!(is(t.accept(typo), FdError::NotRootAuthority), "the earlier proposal is gone");
    // cancelled, then proposed again
    t.propose(old, Pubkey::default()).unwrap();
    assert!(is(t.accept(new), FdError::NotRootAuthority));
    assert!(is(t.accept(Pubkey::default()), FdError::NotRootAuthority));
    t.propose(old, new).unwrap();
    // the proposed key named but not signing
    let mut a = [Acc { signer: false, ..signer(new) }, t.config.clone(), w(t.root_auth.clone())];
    assert!(run(&mut a, fairdrum::instruction::AcceptRootAuthority {}.data()).is_err());
    // a record at another address
    let mut a = [signer(new), t.config.clone(), w(Acc { key: Pubkey::new_unique(), ..t.root_auth.clone() })];
    assert!(is_anchor(run(&mut a, fairdrum::instruction::AcceptRootAuthority {}.data()), anchor_lang::error::ErrorCode::ConstraintSeeds));
    t.accept(new).expect("the proposed key accepts");
    assert_eq!((t.ra().authority, t.ra().pending), (new, Pubkey::default()));
    // replays and the old key
    assert!(is(t.accept(new), FdError::NotRootAuthority), "accept cannot be replayed");
    assert!(is(t.accept(old), FdError::NotRootAuthority));
    assert!(is(t.propose(old, old), FdError::NotRootAuthority), "the old key cannot take the right back");
    assert!(is(t.set_root(old), FdError::NotRootAuthority), "the old key no longer sets the root");
    t.set_root(new).expect("the new key sets the root");
    // a key with another role cannot become the root authority even by hand-over
    for k in [admin, t.promo] {
        t.propose(new, k).unwrap();
        assert!(is(t.accept(k), FdError::BadParams));
        assert_eq!(t.ra().authority, new);
    }
    // Audit No. 16, C-2: the administrator cannot BLOCK a hand-over. A key he merely proposed as the next administrator, or
    // named guardian, is not a key "with another role in force": the hand-over to it goes through.
    let (blocked_a, blocked_b) = (Pubkey::new_unique(), Pubkey::new_unique());
    let mut cfg = t.cfg();
    cfg.pending_dev = blocked_a;
    {
        let mut d = Vec::new();
        cfg.try_serialize(&mut d).unwrap();
        d.resize(8 + Config::INIT_SPACE, 0);
        t.config.data = d;
    }
    assert!(fairdrum::root_key_allowed_on_handover(&cfg, &blocked_a) && !fairdrum::root_key_allowed(&cfg, &blocked_a));
    t.propose(new, blocked_a).unwrap();
    t.accept(blocked_a).expect("a pending administrator proposal does not stop the hand-over");
    assert_eq!((t.ra().authority, t.ra().pending), (blocked_a, Pubkey::default()));
    cfg.guardian = blocked_b;
    {
        let mut d = Vec::new();
        cfg.try_serialize(&mut d).unwrap();
        d.resize(8 + Config::INIT_SPACE, 0);
        t.config.data = d;
    }
    t.propose(blocked_a, blocked_b).unwrap();
    t.accept(blocked_b).expect("naming the key guardian does not stop the hand-over");
    assert_eq!(t.ra().authority, blocked_b);
    // the entry check of set_root_authority is unchanged: the upgrade authority cannot NAME such a key
    assert!(!fairdrum::root_key_allowed(&cfg, &blocked_b));
}

fn token_account(key: Pubkey, owner: Pubkey, amount: u64) -> Acc {
    // SPL Token account, 165 bytes: mint, owner, amount, delegate (4 + 32), state, is_native (4 + 8), delegated amount, close authority (4 + 32)
    let mut d = vec![0u8; 165];
    d[0..32].copy_from_slice(spl_token::native_mint::ID.as_ref());
    d[32..64].copy_from_slice(owner.as_ref());
    d[64..72].copy_from_slice(&amount.to_le_bytes());
    d[108] = 1; // initialized
    acc(key, spl_token::ID, d)
}

/// The root key can do nothing else. Every instruction of the program that asks for a privileged signer is sent with the root
/// key in that place and is rejected; the same accounts with the rightful key pass (so the rejection is about the key).
/// The remaining instructions ask for no privilege: there the root key is an ordinary wallet like any other.
#[test]
fn close_the_root_key_has_no_other_power() {
    let t = world(true, true);
    let root = t.root_key;
    // (name, instruction data, the rightful signer) for the instructions whose accounts are (privileged signer, config)
    let admin_only: Vec<(&str, Vec<u8>)> = vec![
        ("set_jackpot_starts", fairdrum::instruction::SetJackpotStarts { jackpot_start: 200_000_000, super_start: 2_000_000_000 }.data()),
        ("rescale_pools", fairdrum::instruction::RescalePools { to_prize: 1, to_jackpot: 0, to_super: 0 }.data()),
        ("set_paused", fairdrum::instruction::SetPaused { paused: true }.data()),
        ("set_paused(false)", fairdrum::instruction::SetPaused { paused: false }.data()),
        ("set_guardian", fairdrum::instruction::SetGuardian { guardian: root }.data()),
        ("propose_dev_authority", fairdrum::instruction::ProposeDevAuthority { new_authority: root }.data()),
    ];
    for (name, data) in admin_only {
        let mut a = [signer(root), w(t.config.clone())];
        assert!(is(run(&mut a, data.clone()), FdError::Unauthorized), "{name}: the root key must be rejected");
        let mut a = [signer(t.admin), w(t.config.clone())];
        run(&mut a, data).unwrap_or_else(|e| panic!("{name}: the administrator must pass: {e:?}"));
    }
    // guard_pause: neither the administrator nor the guardian
    let mut a = [signer(root), w(t.config.clone())];
    assert!(is(run(&mut a, fairdrum::instruction::GuardPause {}.data()), FdError::Unauthorized));
    let mut a = [signer(t.guardian), w(t.config.clone())];
    run(&mut a, fairdrum::instruction::GuardPause {}.data()).expect("the guardian pauses");
    // accept_dev_authority: not proposed
    let mut a = [signer(root), w(t.config.clone())];
    assert!(is(run(&mut a, fairdrum::instruction::AcceptDevAuthority {}.data()), FdError::Unauthorized));
    // set_root_authority: the root key is not the upgrade authority
    let mut t2 = world(true, true);
    let k = t2.root_key;
    assert!(is(t2.set_root_authority(k, Pubkey::new_unique()), FdError::Unauthorized));

    // instructions with token accounts: Anchor's account validation of the instruction (the place of the permission check)
    let vault = token_account(Pubkey::find_program_address(&[SEED_VAULT], &fairdrum::ID).0, config_key(), 100_000_000_000);
    let dest = token_account(Pubkey::new_unique(), root, 0);
    let token_program = Acc { executable: true, ..acc(spl_token::ID, Pubkey::default(), vec![]) };
    macro_rules! validate {
        ($ty:ty, $bumps:ty, $accs:expr) => {{
            let mut c: Vec<Acc> = $accs;
            let infos: Vec<AccountInfo> = c.iter_mut().map(|a| AccountInfo::new(&a.key, a.signer, a.writable, &mut a.lamports, &mut a.data[..], &a.owner, a.executable)).collect();
            let mut rest = &infos[..];
            let r: R = <$ty>::try_accounts(&fairdrum::ID, &mut rest, &[], &mut <$bumps>::default(), &mut BTreeSet::new()).map(|_| ()).map_err(ProgramError::from);
            r
        }};
    }
    let money = |who: Pubkey| vec![signer(who), w(t.config.clone()), w(vault.clone()), w(dest.clone()), token_program.clone()];
    assert!(is(validate!(fairdrum::ClaimDev, fairdrum::ClaimDevBumps, money(root)), FdError::Unauthorized), "claim_dev: the root key must be rejected");
    validate!(fairdrum::ClaimDev, fairdrum::ClaimDevBumps, money(t.admin)).expect("claim_dev: the administrator passes");
    assert!(is(validate!(fairdrum::PromoPay, fairdrum::PromoPayBumps, money(root)), FdError::Unauthorized), "promo_pay: the root key must be rejected");
    validate!(fairdrum::PromoPay, fairdrum::PromoPayBumps, money(t.promo)).expect("promo_pay: the launch-fund key passes");
    // init_unwrap: administrator only
    let mut mint_data = vec![0u8; 82]; // SPL Token mint: authority (4 + 32), supply, decimals, initialized, freeze authority (4 + 32)
    mint_data[44] = 9;
    mint_data[45] = 1;
    let unwrap_acc = Acc { lamports: 0, writable: true, ..acc(Pubkey::find_program_address(&[SEED_UNWRAP], &fairdrum::ID).0, SYSTEM, vec![]) };
    let r = validate!(fairdrum::InitUnwrap, fairdrum::InitUnwrapBumps, vec![signer(root), t.config.clone(), acc(spl_token::native_mint::ID, spl_token::ID, mint_data), unwrap_acc, token_program.clone(), Acc { executable: true, ..acc(SYSTEM, Pubkey::default(), vec![]) }]);
    // Anchor creates the service account before it compares the signer, and off a node nothing can be created: the exact
    // error (Unauthorized) is checked on a node (scripts/sol-roundtest.ts, tests/local/close-test.ts); here: rejected
    assert!(r.is_err(), "init_unwrap: the root key must be rejected");
}

/// The new account does not touch the existing ones: sizes as before, and the record itself is 73 bytes.
#[test]
fn close_layout_is_unchanged() {
    assert_eq!(8 + Config::INIT_SPACE, 1303);
    assert_eq!(8 + fairdrum::state::Player::INIT_SPACE, 272);
    assert_eq!(8 + fairdrum::state::Session::INIT_SPACE, 89);
    assert_eq!(8 + RootAuth::INIT_SPACE, 73);
    // error numbers: the existing ones did not move, the new one is appended
    assert_eq!(u32::from(FdError::UnwrapFailed), 6034);
    assert_eq!(u32::from(FdError::NotRootAuthority), 6035);
    assert_eq!(u32::from(FdError::Unauthorized), 6019);
    assert_eq!(u32::from(FdError::WithdrawLocked), 6028);
}
