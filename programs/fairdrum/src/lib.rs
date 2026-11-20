//! FAIRDRUM — an open drum game on Solana.
//!
//! Principles:
//!  * The code has no function that changes the chances, stops payouts or takes the pools. The only way to change the rules is
//!    to load a new program version with the upgrade key, while it has not been revoked (see docs/AUDIT.md).
//!  * Randomness is a "fair draw" from two independent parts: a house secret committed IN ADVANCE (the root of the secret
//!    tree is stored in the contract, the round secret is revealed at settlement and checked against the root), and the hash of the block
//!    the bet landed in (nobody knows it at bet time). The house does not know the block hash, the player and the validator do not know the secret.
//!    There is no external oracle: a measurement on 13.11.2026 showed that the ORAO number for any request can be obtained in advance on the test network.
//!  * All accounting of the game and the players is kept on the blockchain; the site is static files with no database.
//!  * Launch from zero players: no airdrop, no lists, registration is open to everyone.
//!  * The only "manual" wallets: dev_authority (collects its share — S_DEV = 618 of every 10,000 of an executed spin, 6.991% of paid bets; 10% in the program deployed before upgrade No. 2 —, sets the starting amounts of
//!    the jackpots, can pause acceptance of NEW bets) and promo_authority (collects the launch surplus). Every payout is public (PromoPaid / Withdrawn events). Everything else runs without people.
//!  * The root of the house secrets is set only by the holder of the program's upgrade authority (upgrade No. 2, audit No. 10):
//!    the administrator key alone cannot replace the source of the round secrets.
//!  * Closing the program (owner's decision, option A): the upgrade authority can name a dedicated ROOT AUTHORITY key
//!    (`set_root_authority`, account `RootAuth`). From then on only that key sets the secrets root, and it can do nothing else.
//!    After the upgrade authority is revoked the code can never change again, while the root authority keeps the game supplied with
//!    secrets and can hand its right over in two steps (`propose_root_authority` / `accept_root_authority`).

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

pub mod consts;
pub mod engine;
pub mod errors;
pub mod math;
pub mod state;

use consts::*;
use errors::FdError;
use state::*;

// THE MAINNET ADDRESS of the program (the address the players' funds live under). It is committed as it is deployed: the
// published source, the IDL (`idl/*.json`), `Anchor.toml` and the release build all carry this one address, and the deploy
// script (`scripts/deploy-program.sh`) REFUSES to build when the program key file, the launch settings and this line disagree
// (it no longer rewrites this line). A build whose declared address differs from the address it runs under rejects every
// instruction (DeclaredProgramIdMismatch). The local harness (`tests/local/common.sh`) puts its own test address here, in a
// temporary copy, for the builds with shortened time limits: such a build can therefore never work at the mainnet address.
declare_id!("5ww3w4HrQrrrPcjfxVRvjLpL5vBJqLt7M5ThsnEkkngf");

#[program]
pub mod fairdrum {
    use super::*;

    /// Once, only by the holder of the program's upgrade authority. The game currency is wrapped SOL: the mint passed here
    /// must be the native mint (checked below).
    pub fn initialize(ctx: Context<Initialize>, params: InitParams) -> Result<()> {
        let mint = &ctx.accounts.mint;
        require!(mint.decimals == DECIMALS, FdError::BadParams);
        // The game currency is wrapped SOL only: a token of one's own cannot be slipped in instead of SOL.
        require_keys_eq!(mint.key(), anchor_spl::token::spl_token::native_mint::ID, FdError::BadParams);
        require!(params.start_ts > 0, FdError::BadParams);
        require!(params.dev_authority != Pubkey::default() && params.promo_authority != Pubkey::default(), FdError::BadParams);
        require!(params.init_prize >= params.prize_buffer, FdError::BadParams);

        let cfg = &mut ctx.accounts.config;
        // the account has just been created and is zero-filled: we do not build a separate Config "template" on the stack (it does not fit into the frame)
        cfg.bump = ctx.bumps.config;
        cfg.vault_bump = ctx.bumps.vault;
        cfg.mint = ctx.accounts.mint.key();
        cfg.vault = ctx.accounts.vault.key();
        cfg.dev_authority = params.dev_authority;
        cfg.promo_authority = params.promo_authority;
        cfg.start_ts = params.start_ts;
        cfg.init_act = params.init_act;
        cfg.init_prize = params.init_prize;
        cfg.init_seed_reserve = params.init_seed_reserve;
        cfg.init_jackpot = params.init_jackpot;
        cfg.init_super = params.init_super;
        cfg.prize_buffer = params.prize_buffer;
        cfg.den = DEN_MIN;
        cfg.epoch_start = params.start_ts;
        Ok(())
    }

    /// After the tokens have been sent to the vault. Anyone can call it, once.
    pub fn fund_pools(ctx: Context<FundPools>) -> Result<()> {
        engine::fund(&mut ctx.accounts.config, ctx.accounts.vault.amount)
    }

    /// Registration: open to everyone right after launch. The game has no invitations or referrals.
    pub fn register(ctx: Context<Register>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let cfg = &mut ctx.accounts.config;
        let number = engine::assign_number(cfg)?;
        engine::init_player(&mut ctx.accounts.player, ctx.accounts.user.key(), ctx.bumps.player, number, now);
        emit!(Registered { owner: ctx.accounts.user.key(), number });
        Ok(())
    }

    /// "I am human": a day check-in without a spin. Extends the streak.
    pub fn checkin(ctx: Context<Checkin>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let t = engine::touch(&mut ctx.accounts.player, now);
        if t.burned {
            emit!(Burned { owner: ctx.accounts.user.key(), streak: t.old_streak, run_spins: t.old_run });
        }
        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(amount > 0, FdError::BadAmount);
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.user_ata.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount,
        )?;
        let before = owed_of(&ctx.accounts.player);
        engine::deposit(&mut ctx.accounts.player, amount, Clock::get()?.unix_timestamp)?;
        track_owed(&mut ctx.accounts.config, before, owed_of(&ctx.accounts.player))
    }

    /// Credit SOL from the player's personal deposit address to their game balance. The address is derived from the player's wallet,
    /// nobody holds a key to it; SOL can be sent to it from anywhere (an exchange, another wallet). Anyone can send
    /// the command — it can do only one thing: move SOL from the owner's address to that same owner's balance.
    /// The sender of the command is reimbursed the network fee; if the player's wallet is empty, a small part goes to it for fees.
    pub fn deposit_sweep(ctx: Context<DepositSweep>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let player_bump = ctx.bumps.player;
        let a = ctx.accounts;
        let owner = a.owner.key();
        let total = a.deposit.lamports();
        require!(total >= SWEEP_MIN, FdError::BadAmount);
        // First deposit of a new player: the game account is created right here. The network's rent for the account record was paid by the sender
        // of the command — it is reimbursed to them from the deposit, so a new player needs no SOL in the wallet to start.
        // The "account just created" marker is registration number 0 (a created account always has ≥ 1), not a zero owner:
        // the zero address is the system program's address, and for it the previous marker would stay true forever (audit No. 8, A4).
        require_keys_neq!(owner, Pubkey::default(), FdError::BadParams);
        let created = a.player.reg_no == 0;
        let mut refund = SWEEP_FEE_REFUND;
        if created {
            let number = engine::assign_number(&mut a.config)?;
            engine::init_player(&mut a.player, owner, player_bump, number, now);
            refund += Rent::get()?.minimum_balance(8 + Player::INIT_SPACE);
            emit!(Registered { owner, number });
        }
        let drip = if a.owner.lamports() < WALLET_FLOOR { WALLET_DRIP } else { 0 };
        let amount = total.checked_sub(refund + drip).ok_or(FdError::BadAmount)?;
        require!(amount > 0, FdError::BadAmount);
        let bump = [ctx.bumps.deposit];
        let seeds: &[&[u8]] = &[SEED_DEPOSIT, owner.as_ref(), &bump];
        for (to, lamports) in [(a.payer.to_account_info(), refund), (a.owner.to_account_info(), drip), (a.vault.to_account_info(), amount)] {
            if lamports > 0 {
                anchor_lang::system_program::transfer(
                    CpiContext::new_with_signer(a.system_program.key(), anchor_lang::system_program::Transfer { from: a.deposit.to_account_info(), to }, &[seeds]),
                    lamports,
                )?;
            }
        }
        // SOL has landed on the vault account: we "wrap" it so that the vault amount grows
        token::sync_native(CpiContext::new(a.token_program.key(), token::SyncNative { account: a.vault.to_account_info() }))?;
        let before = owed_of(&a.player);
        engine::deposit(&mut a.player, amount, now)?;
        track_owed(&mut a.config, before, owed_of(&a.player))?;
        emit!(DepositSwept { owner, amount, drip, payer: a.payer.key(), created });
        Ok(())
    }

    /// Contribute one's own funds to the jackpot seed reserve (anyone can). They cannot be taken back:
    /// from the reserve funds go only into the jackpot pots, that is, to players.
    pub fn donate(ctx: Context<Donate>, amount: u64) -> Result<()> {
        engine::donate(&mut ctx.accounts.config, amount)?;
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                Transfer {
                    from: ctx.accounts.user_ata.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount,
        )
    }

    /// Starting amounts of the jackpot and the super jackpot (administrator only): a pot is topped up to them from the reserve after a win.
    /// They take effect after 24 hours; bets placed earlier are settled at the previous amounts.
    pub fn set_jackpot_starts(ctx: Context<MiniSeedTopup>, jackpot_start: u64, super_start: u64) -> Result<()> {
        engine::set_starts(&mut ctx.accounts.config, jackpot_start, super_start, Clock::get()?.unix_timestamp)?;
        emit!(AdminAction { kind: 2, a: jackpot_start, b: super_start, key: ctx.accounts.dev.key() });
        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, amount: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let before = owed_of(&ctx.accounts.player);
        let (net, fee, burn) = engine::withdraw(&mut ctx.accounts.config, &mut ctx.accounts.player, amount, now)?;
        track_owed(&mut ctx.accounts.config, before, owed_of(&ctx.accounts.player))?;
        // vault protection: after the payout it must hold no less than the contract owes the other players and the pots
        solvent(&ctx.accounts.config, ctx.accounts.vault.amount.saturating_sub(net))?;
        let bump = [ctx.accounts.config.bump];
        let seeds: &[&[u8]] = &[SEED_CONFIG, &bump];
        if net > 0 {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.key(),
                    Transfer {
                        from: ctx.accounts.vault.to_account_info(),
                        to: ctx.accounts.user_ata.to_account_info(),
                        authority: ctx.accounts.config.to_account_info(),
                    },
                    &[seeds],
                ),
                net,
            )?;
        }
        debug_assert_eq!(burn, 0); // the SOL version has no burning
        emit!(Withdrawn { owner: ctx.accounts.user.key(), amount, fee });
        Ok(())
    }

    /// Withdrawal in regular SOL — without "wrapped SOL" in the player's wallet (upgrade No. 2, change C; site audit S9-01).
    /// The previous `withdraw` command pays to the player's token account, which the site must first create for ~0.00204 SOL from
    /// the player's wallet; a built-in wallet may not have those funds. Here the player needs only the network fee.
    ///
    /// The accounting, the withdrawal rules and the vault protection are the same as for `withdraw` (the same `engine::withdraw`, the same `solvent`).
    /// Only the delivery of the funds differs: vault → service account of wrapped SOL (its owner is the config account) →
    /// closing the service account into the player's wallet (this turns wrapped SOL into regular SOL) → the service account is re-created
    /// on the rent deposit the player returned. All in one command: either the player received exactly `net` in regular SOL and the service account
    /// is empty and ready again, or nothing changed.
    pub fn withdraw_sol(ctx: Context<WithdrawSol>, amount: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let unwrap_bump = [ctx.bumps.unwrap_account];
        let a = ctx.accounts;
        let before = owed_of(&a.player);
        let (net, fee, burn) = engine::withdraw(&mut a.config, &mut a.player, amount, now)?;
        track_owed(&mut a.config, before, owed_of(&a.player))?;
        // vault protection: after the payout it must hold no less than the contract owes the other players and the pots
        solvent(&a.config, a.vault.amount.saturating_sub(net))?;
        debug_assert_eq!(burn, 0); // the SOL version has no burning
        if net > 0 {
            let user_before = a.user.lamports();
            let bump = [a.config.bump];
            let cfg_seeds: &[&[u8]] = &[SEED_CONFIG, &bump];
            let unwrap_seeds: &[&[u8]] = &[SEED_UNWRAP, &unwrap_bump];
            // 1. vault → service account: exactly net (the only place where the vault is touched)
            token::transfer(
                CpiContext::new_with_signer(
                    a.token_program.key(),
                    Transfer { from: a.vault.to_account_info(), to: a.unwrap_account.to_account_info(), authority: a.config.to_account_info() },
                    &[cfg_seeds],
                ),
                net,
            )?;
            // 2. the service account is closed into the player's wallet: the player receives net and the service account's rent deposit
            token::close_account(CpiContext::new_with_signer(
                a.token_program.key(),
                token::CloseAccount { account: a.unwrap_account.to_account_info(), destination: a.user.to_account_info(), authority: a.config.to_account_info() },
                &[cfg_seeds],
            ))?;
            // 3. the player returns the rent deposit, the service account is re-created (the same way Anchor creates an account whose
            //    address already holds lamports: transfer, then allocate space and assign to the token program)
            let rent = Rent::get()?.minimum_balance(TOKEN_ACCOUNT_LEN as usize);
            let info = a.unwrap_account.to_account_info();
            let need = rent.saturating_sub(info.lamports());
            if need > 0 {
                anchor_lang::system_program::transfer(
                    CpiContext::new(a.system_program.key(), anchor_lang::system_program::Transfer { from: a.user.to_account_info(), to: info.clone() }),
                    need,
                )?;
            }
            if *info.owner == anchor_lang::system_program::ID {
                anchor_lang::system_program::allocate(
                    CpiContext::new_with_signer(a.system_program.key(), anchor_lang::system_program::Allocate { account_to_allocate: info.clone() }, &[unwrap_seeds]),
                    TOKEN_ACCOUNT_LEN,
                )?;
                anchor_lang::system_program::assign(
                    CpiContext::new_with_signer(a.system_program.key(), anchor_lang::system_program::Assign { account_to_assign: info.clone() }, &[unwrap_seeds]),
                    &a.token_program.key(),
                )?;
            }
            token::initialize_account3(CpiContext::new(
                a.token_program.key(),
                token::InitializeAccount3 { account: info, mint: a.mint.to_account_info(), authority: a.config.to_account_info() },
            ))?;
            // 4. result: the player received no less than net in regular SOL (the network fee is charged before the command and is not included here)
            require!(a.user.lamports() >= user_before.checked_add(net).ok_or(FdError::MathOverflow)?, FdError::UnwrapFailed);
        }
        emit!(Withdrawn { owner: a.user.key(), amount, fee });
        Ok(())
    }

    /// Once after the upgrade: create the service account of wrapped SOL for `withdraw_sol`. Administrator only (audit No. 10,
    /// P10-3): this is the switch that turns `withdraw_sol` on, and it must stay in the owner's hands until the instruction has been
    /// rehearsed on the mainnet token program. The administrator pays the network's rent deposit ≈0.00204 SOL; the account's address
    /// and owner are set by the program and cannot be chosen. Until the account is created, `withdraw_sol` is rejected and
    /// the previous `withdraw` works as before.
    pub fn init_unwrap(_ctx: Context<InitUnwrap>) -> Result<()> {
        Ok(())
    }

    /// Round step 1: the bet is debited, the round gets the next house secret number, the bet's block and
    /// the secret root in effect are remembered. Nobody knows the outcome at this moment: the secret is not yet revealed and the block is not yet closed.
    pub fn spin_commit(ctx: Context<SpinCommit>, n: u16, use_free: bool, mode: u8) -> Result<()> {
        let clock = Clock::get()?;
        require!(!ctx.accounts.config.paused, FdError::Paused);
        // smallest round — as on the site (0.1 SOL); settlement and refund of bets already placed do not check the size
        require!(n >= MIN_BATCH, FdError::BadCount);
        let owner = ctx.accounts.player.owner;
        // The commit can be signed by the owner or by their session key (within its lifetime and budget).
        if ctx.accounts.user.key() != owner {
            let s = ctx.accounts.session.as_mut().ok_or(FdError::Unauthorized)?;
            require_keys_eq!(s.owner, owner, FdError::Unauthorized);
            require_keys_eq!(s.key, ctx.accounts.user.key(), FdError::Unauthorized);
            require!(clock.unix_timestamp < s.expiry_ts, FdError::SessionExpired);
            // a session key bets at most SESSION_MAX_BATCH per round; more — only with a wallet signature
            require!(n <= SESSION_MAX_BATCH, FdError::SessionLimit);
            let stake = if use_free { 0 } else { (n as u64) * COST };
            require!(stake <= s.limit, FdError::SessionLimit);
            s.limit -= stake;
        }
        // secret number: the ordinal of the bet since the root was set; the tree holds 2^depth secrets
        let cfg = &ctx.accounts.config;
        let leaf = cfg.round_counter.checked_sub(cfg.house_start).ok_or(FdError::MathOverflow)?;
        require!(cfg.house_depth > 0 && cfg.house_depth <= HOUSE_MAX_DEPTH && leaf < (1u64 << cfg.house_depth), FdError::HouseExhausted);
        let root = Pubkey::new_from_array(cfg.house_root);
        let before = owed_of(&ctx.accounts.player);
        let t = engine::commit_mode(&mut ctx.accounts.config, &mut ctx.accounts.player, clock.unix_timestamp, clock.slot, n, mode, use_free, root)?;
        track_owed(&mut ctx.accounts.config, before, owed_of(&ctx.accounts.player))?;
        ctx.accounts.player.p_leaf = leaf;
        ctx.accounts.player.p_sealed = false;
        ctx.accounts.player.p_salt = [0u8; 32];
        ctx.accounts.config.round_counter = ctx.accounts.config.round_counter.checked_add(1).ok_or(FdError::MathOverflow)?;
        if t.burned {
            emit!(Burned { owner, streak: t.old_streak, run_spins: t.old_run });
        }
        emit!(SpinCommitted { owner, n, free: use_free, slot: clock.slot });
        Ok(())
    }

    /// Step 2: settlement. The caller (usually the house keeper) reveals the round secret and the path to the root; the contract checks
    /// the secret against the root recorded at bet time, mixes it with the hash of the bet's block and computes the outcome. The secret cannot be swapped:
    /// only one fits the root. After the round's deadline (≈20 minutes) the pots are no longer played in it: this way the owner
    /// of the secret cannot wait for a pot to grow.
    pub fn spin_settle(ctx: Context<SpinSettle>, secret: [u8; 32], proof: Vec<[u8; 32]>) -> Result<()> {
        let clock = Clock::get()?;
        require!(ctx.accounts.player.pending, FdError::NoPending);
        require!(clock.slot > ctx.accounts.player.p_commit_slot, FdError::SettleTooEarly);
        let leaf = ctx.accounts.player.p_leaf;
        require!(proof.len() <= HOUSE_MAX_DEPTH as usize && leaf < (1u64 << proof.len()), FdError::BadProof);
        require!(math::house_root_of(leaf, &secret, &proof) == ctx.accounts.player.p_root.to_bytes(), FdError::BadProof);
        // The salt is the hash of the bet's block. If it has not been recorded yet (spin_seal), we take it now; if the bet's block has already dropped out of the list
        // of recent blocks (≈3.5 minutes), settlement is impossible: without the salt the outcome would be determined by the house secret alone.
        seal_if_possible(&mut ctx.accounts.player, &ctx.accounts.slot_hashes)?;
        require!(ctx.accounts.player.p_sealed, FdError::NotSealed);
        let salt = ctx.accounts.player.p_salt;
        let value = math::salted(secret, &salt);
        let late = clock.slot >= ctx.accounts.player.p_commit_slot + PENDING_EXPIRY_SLOTS;
        emit!(RoundRevealed { owner: ctx.accounts.player.owner, leaf, root: ctx.accounts.player.p_root.to_bytes(), secret, salt, commit_slot: ctx.accounts.player.p_commit_slot });
        finish_spin(ctx.accounts, Some(value), clock.unix_timestamp, late)
    }

    /// Refund of the bet for a round the house did not complete. Two cases:
    ///  * the round was neither settled nor sealed while the hash of the bet's block was available (≈3.5 minutes) — it will never have an outcome;
    ///  * the round is sealed, but the house did not reveal its secret for 24 hours.
    /// The bet is returned in full to the game balance. Anyone can call it. The player cannot influence this: they do not know
    /// the secret and cannot obstruct settlement. Every refund increments the public counter Config.refunds.
    pub fn spin_expire(ctx: Context<SpinSettle>) -> Result<()> {
        let clock = Clock::get()?;
        require!(ctx.accounts.player.pending, FdError::NoPending);
        seal_if_possible(&mut ctx.accounts.player, &ctx.accounts.slot_hashes)?;
        let age = clock.slot.saturating_sub(ctx.accounts.player.p_commit_slot);
        if ctx.accounts.player.p_sealed {
            require!(age > REVEAL_TIMEOUT_SLOTS, FdError::NotExpired);
        } else {
            require!(age > SEAL_WINDOW_SLOTS, FdError::NotExpired);
        }
        let a = ctx.accounts;
        let owner = a.player.owner;
        let before = owed_of(&a.player);
        engine::refund(&mut a.config, &mut a.player, clock.unix_timestamp)?;
        track_owed(&mut a.config, before, owed_of(&a.player))?;
        solvent(&a.config, a.vault.amount)?;
        emit!(SpinSettled { owner, spins: 0, extras: 0, gold: 0, prizes: 0, best_prize: 0, mini_hits: 0, mini_won: 0, jackpot_hits: 0, jackpot_total: 0, super_hits: 0, super_total: 0, forfeited: false, refunded: true });
        Ok(())
    }

    /// Seal a round: record the hash of the bet's block while it is available (≈3.5 minutes after the bet). Anyone can call it,
    /// any number of times: the value is uniquely determined by the bet's block. Needed if settlement is delayed (the house
    /// keeper is unavailable): a sealed round can be settled later, an unsealed one cannot. Usually settlement seals it itself.
    pub fn spin_seal(ctx: Context<SpinSeal>) -> Result<()> {
        require!(ctx.accounts.player.pending, FdError::NoPending);
        require!(Clock::get()?.slot > ctx.accounts.player.p_commit_slot, FdError::SettleTooEarly);
        seal_if_possible(&mut ctx.accounts.player, &ctx.accounts.slot_hashes)?;
        require!(ctx.accounts.player.p_sealed, FdError::NotSealed);
        Ok(())
    }

    /// Once a week updates the weekly spin statistics. Moves no funds (the bank's surplus is released by settlements, not here). Called automatically on spins.
    pub fn roll_epoch(ctx: Context<RollEpoch>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        if let Some(e) = engine::roll_epoch_if_due(&mut ctx.accounts.config, now) {
            emit!(e);
        }
        Ok(())
    }

    /// Administrator's share: S_DEV = 618 of every 10,000 of an executed spin = 6.991% of paid bets (it was 884 = 10% before
    /// upgrade No. 2), accumulates in dev_pool (dev_authority).
    pub fn claim_dev(ctx: Context<ClaimDev>, amount: u64) -> Result<()> {
        let cfg = &mut ctx.accounts.config;
        require!(amount > 0 && amount <= cfg.dev_pool, FdError::BadAmount);
        cfg.dev_pool -= amount;
        solvent(cfg, ctx.accounts.vault.amount.saturating_sub(amount))?;
        emit!(AdminAction { kind: 1, a: amount, b: 0, key: ctx.accounts.dev_ata.owner });
        vault_transfer(&ctx.accounts.config, &ctx.accounts.vault, &ctx.accounts.dev_ata, &ctx.accounts.token_program, amount)
    }

    /// Payout from the promotion fund (invitations, monthly ranking, advertising). Every payout is public.
    pub fn promo_pay(ctx: Context<PromoPay>, amount: u64) -> Result<()> {
        let cfg = &mut ctx.accounts.config;
        require!(amount > 0 && amount <= cfg.promo_pool, FdError::BadAmount);
        cfg.promo_pool -= amount;
        solvent(cfg, ctx.accounts.vault.amount.saturating_sub(amount))?;
        emit!(PromoPaid { to: ctx.accounts.dest_ata.owner, amount });
        vault_transfer(&ctx.accounts.config, &ctx.accounts.vault, &ctx.accounts.dest_ata, &ctx.accounts.token_program, amount)
    }

    /// Move of the administrator's funds from the seed reserve into the games' prize bank; moving into the pots is not allowed (administrator only).
    // The line above is published in the IDL and is kept as it was. Since the re-verification of 18.11.2026 the move is
    // accepted only while the bank is below BANK_KEEP and at most up to it (engine::rescale), so it cannot undo the release
    // of the bank's surplus; a refused amount fails with BadAmount.
    pub fn rescale_pools(ctx: Context<MiniSeedTopup>, to_prize: u64, to_jackpot: u64, to_super: u64) -> Result<()> {
        engine::rescale(&mut ctx.accounts.config, to_prize, to_jackpot, to_super)?;
        emit!(AdminAction { kind: 3, a: to_prize, b: 0, key: ctx.accounts.dev.key() });
        Ok(())
    }

    /// Pause or resume acceptance of NEW bets (administrator only) — in case of a failure or an incident.
    /// Bets already placed are settled as usual; payouts and withdrawals do not depend on the pause.
    pub fn set_paused(ctx: Context<MiniSeedTopup>, paused: bool) -> Result<()> {
        ctx.accounts.config.paused = paused;
        emit!(AdminAction { kind: 4, a: paused as u64, b: 0, key: ctx.accounts.dev.key() });
        Ok(())
    }

    /// Set the root of the house secret tree. Applies to SUBSEQUENT bets: every bet already
    /// placed has its own root recorded in its account, and a new root does not affect it. Needed at launch, when the secrets
    /// are running out, and if the secrets file may have fallen into the wrong hands.
    ///
    /// Who signs (exactly one party at any time; never the administrator: upgrade No. 2, audit No. 10, P10-1):
    ///  * the `RootAuth` account EXISTS (created by `set_root_authority`): only the root authority recorded in it. The upgrade
    ///    authority is then refused here, and the program data account is not read at all, so this path does not depend on the
    ///    upgrade authority or on the loader in any way and keeps working after the upgrade authority is revoked;
    ///  * the `RootAuth` account does not exist: the holder of the program's upgrade authority, as before. If the upgrade
    ///    authority is revoked in THAT state, the root can never be changed again (bets stop when the secrets run out).
    /// Whoever sets the root knows the secrets of every later round and can settle rounds selectively.
    pub fn set_house_root(ctx: Context<SetHouseRoot>, root: [u8; 32], depth: u8) -> Result<()> {
        let signer = ctx.accounts.authority.key();
        let ra = ctx.accounts.root_auth.to_account_info();
        if *ra.owner == crate::ID {
            // Only this program can create an account it owns at this address, and it does so only in `set_root_authority`.
            // An unreadable record is an error, never a fall-back to the other path.
            let st = RootAuth::try_deserialize(&mut &ra.try_borrow_data()?[..])?;
            require!(st.authority != Pubkey::default() && st.authority == signer, FdError::NotRootAuthority);
        } else {
            let pd = ctx.accounts.program_data.to_account_info();
            require!(is_own_program_data(&crate::ID, pd.key), FdError::Unauthorized);
            require!(*pd.owner == anchor_lang::solana_program::bpf_loader_upgradeable::ID, FdError::Unauthorized);
            let st = ProgramData::try_deserialize(&mut &pd.try_borrow_data()?[..]).map_err(|_| error!(FdError::Unauthorized))?;
            require!(st.upgrade_authority_address == Some(signer), FdError::Unauthorized);
        }
        let cfg = &mut ctx.accounts.config;
        engine::set_root(cfg, root, depth)?;
        emit!(AdminAction { kind: 9, a: depth as u64, b: cfg.round_counter, key: Pubkey::new_from_array(root) });
        Ok(())
    }

    /// Name the ROOT AUTHORITY: the dedicated key that alone may set the secrets root from now on (see `set_house_root`).
    /// Signed by the holder of the program's upgrade authority; creates the `RootAuth` account or replaces the key in it (and
    /// cancels a pending hand-over). Possible only while an upgrade authority exists: after it is revoked the key changes only
    /// by the two-step hand-over below. The key cannot be empty or coincide with the administrator, the proposed administrator,
    /// the launch-fund key or the guardian: the root authority must be a key that can do nothing else.
    pub fn set_root_authority(ctx: Context<SetRootAuthority>, key: Pubkey) -> Result<()> {
        require!(root_key_allowed(&ctx.accounts.config, &key), FdError::BadParams);
        let ra = &mut ctx.accounts.root_auth;
        ra.authority = key;
        ra.pending = Pubkey::default();
        ra.bump = ctx.bumps.root_auth;
        emit!(AdminAction { kind: 10, a: 0, b: 0, key });
        Ok(())
    }

    /// Hand-over of the root authority, step 1: the current root authority names the new key. Pubkey::default() cancels.
    pub fn propose_root_authority(ctx: Context<ProposeRootAuthority>, new_authority: Pubkey) -> Result<()> {
        ctx.accounts.root_auth.pending = new_authority;
        emit!(AdminAction { kind: 11, a: 0, b: 0, key: new_authority });
        Ok(())
    }

    /// Hand-over of the root authority, step 2: the new key confirms it itself, so the right cannot be handed to a keyless
    /// address by a typo (after the program is closed that would stop the game for good when the secrets run out).
    pub fn accept_root_authority(ctx: Context<AcceptRootAuthority>) -> Result<()> {
        let new = ctx.accounts.new_authority.key();
        // Only the roles that are in force are compared (audit No. 16, C-2): a key that the administrator merely PROPOSED as
        // the next administrator, or named guardian, does not stop the hand-over. Otherwise the administrator could block
        // every hand-over of the root authority after the program is closed, when `set_root_authority` no longer exists.
        require!(root_key_allowed_on_handover(&ctx.accounts.config, &new), FdError::BadParams);
        let ra = &mut ctx.accounts.root_auth;
        ra.authority = new;
        ra.pending = Pubkey::default();
        emit!(AdminAction { kind: 12, a: 0, b: 0, key: new });
        Ok(())
    }

    /// Appoint a "guardian" (administrator only): a key that can only pause acceptance of new bets.
    pub fn set_guardian(ctx: Context<MiniSeedTopup>, guardian: Pubkey) -> Result<()> {
        ctx.accounts.config.guardian = guardian;
        emit!(AdminAction { kind: 7, a: 0, b: 0, key: guardian });
        Ok(())
    }

    /// The guardian (or the administrator) pauses acceptance of NEW bets. Only the administrator can lift the pause.
    /// Settlements, payouts and withdrawals do not depend on the pause.
    pub fn guard_pause(ctx: Context<GuardPause>) -> Result<()> {
        let cfg = &mut ctx.accounts.config;
        let who = ctx.accounts.guard.key();
        require!(who == cfg.dev_authority || (cfg.guardian != Pubkey::default() && who == cfg.guardian), FdError::Unauthorized);
        cfg.paused = true;
        emit!(AdminAction { kind: 8, a: 1, b: 0, key: who });
        Ok(())
    }

    /// Changing the administrator key, step 1: the current key names the new one. Pubkey::default() cancels the proposal.
    pub fn propose_dev_authority(ctx: Context<MiniSeedTopup>, new_authority: Pubkey) -> Result<()> {
        ctx.accounts.config.pending_dev = new_authority;
        emit!(AdminAction { kind: 5, a: 0, b: 0, key: new_authority });
        Ok(())
    }

    /// Changing the administrator key, step 2: the new key confirms it itself — this way the rights cannot be handed to a keyless address by mistake.
    /// If the launch fund was under the same key, it moves along with it.
    pub fn accept_dev_authority(ctx: Context<AcceptDev>) -> Result<()> {
        let cfg = &mut ctx.accounts.config;
        let new = ctx.accounts.new_dev.key();
        if cfg.promo_authority == cfg.dev_authority {
            cfg.promo_authority = new;
        }
        cfg.dev_authority = new;
        cfg.pending_dev = Pubkey::default();
        emit!(AdminAction { kind: 6, a: 0, b: 0, key: new });
        Ok(())
    }

    /// Open (or reissue) a game session: with one signature the owner allows a temporary
    /// key to make spins on their behalf — for no longer than secs seconds and no more than limit tokens of bets.
    /// A session key cannot withdraw, deposit or change the wallet: only commit spins.
    pub fn session_open(ctx: Context<SessionOpen>, key: Pubkey, secs: i64, limit: u64) -> Result<()> {
        require!(secs > 0 && secs <= SESSION_MAX_SECS, FdError::BadSession);
        require!(limit > 0 && key != Pubkey::default(), FdError::BadSession);
        let s = &mut ctx.accounts.session;
        s.owner = ctx.accounts.user.key();
        s.key = key;
        s.expiry_ts = Clock::get()?.unix_timestamp + secs;
        s.limit = limit;
        s.bump = ctx.bumps.session;
        Ok(())
    }

    /// Close a session early: the account rent is returned to the owner.
    pub fn session_close(_ctx: Context<SessionClose>) -> Result<()> {
        Ok(())
    }
}

// ---------------- helpers ----------------

/// Address of the account holding the program code in the upgradeable loader: a PDA of the program id.
pub fn program_data_address(program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[program_id.as_ref()], &anchor_lang::solana_program::bpf_loader_upgradeable::ID).0
}

pub fn is_own_program_data(program_id: &Pubkey, candidate: &Pubkey) -> bool {
    program_data_address(program_id) == *candidate
}

/// The root authority must be a key with no other role in the program.
pub fn root_key_allowed(cfg: &Config, key: &Pubkey) -> bool {
    *key != Pubkey::default() && *key != cfg.dev_authority && *key != cfg.pending_dev && *key != cfg.promo_authority && *key != cfg.guardian
}

/// The check of `accept_root_authority`: the new root authority is not the administrator and not the launch-fund key.
/// The pending administrator and the guardian are deliberately not compared here (see the instruction). What this leaves
/// open, and no instruction with the account list of the deployed program can close: `accept_dev_authority` and
/// `set_guardian` do not read the `RootAuth` record, so the administrator role or the guardian role can later be given TO
/// the root key. Both need the administrator's signature; the check before any hand-over is in docs/CLOSING-RU.md.
pub fn root_key_allowed_on_handover(cfg: &Config, key: &Pubkey) -> bool {
    *key != Pubkey::default() && *key != cfg.dev_authority && *key != cfg.promo_authority
}

fn vault_transfer<'info>(
    cfg: &Account<'info, Config>,
    vault: &Account<'info, TokenAccount>,
    to: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
    amount: u64,
) -> Result<()> {
    let bump = [cfg.bump];
    let seeds: &[&[u8]] = &[SEED_CONFIG, &bump];
    token::transfer(
        CpiContext::new_with_signer(
            token_program.key(),
            Transfer { from: vault.to_account_info(), to: to.to_account_info(), authority: cfg.to_account_info() },
            &[seeds],
        ),
        amount,
    )
}

/// Solana system account with the list of hashes of the last 512 blocks
pub const SLOT_HASHES_ID: Pubkey = pubkey!("SysvarS1otHashes111111111111111111111111111");

/// How much the contract owes this player: the game balance, the frozen part of a jackpot and the bet of an unfinished round.
fn owed_of(p: &Player) -> u64 {
    p.balance.saturating_add(p.jp_locked).saturating_add(if p.pending { p.p_stake } else { 0 })
}
/// Updates the total counter of obligations to players after one account changes.
fn track_owed(cfg: &mut Config, before: u64, after: u64) -> Result<()> {
    cfg.owed = cfg.owed.checked_add(after).and_then(|x| x.checked_sub(before)).ok_or(FdError::MathOverflow)?;
    Ok(())
}
/// Vault protection: it must hold no less than the sum of all pots and obligations to players.
/// With correct accounting this is an equality; a violation means a bug or an attack — such an operation is rejected.
fn solvent(cfg: &Config, vault_amount: u64) -> Result<()> {
    let pools = [cfg.act_pool, cfg.prize_pool, cfg.jackpot_pool, cfg.mini_pool, cfg.super_pool, cfg.seed_pool, cfg.seed_reserve, cfg.dev_pool, cfg.promo_pool]
        .iter()
        .try_fold(0u64, |a, x| a.checked_add(*x))
        .ok_or(FdError::MathOverflow)?;
    require!(vault_amount >= pools.checked_add(cfg.owed).ok_or(FdError::MathOverflow)?, FdError::SolvencyGuard);
    Ok(())
}

/// Records the round's salt if it is not yet recorded and the bet's block is still in the list of recent blocks.
fn seal_if_possible(player: &mut Account<Player>, slot_hashes: &UncheckedAccount) -> Result<()> {
    if player.p_sealed {
        return Ok(());
    }
    let data = slot_hashes.try_borrow_data()?;
    if let Some(h) = math::slot_hash_of(&data, player.p_commit_slot) {
        player.p_salt = h;
        player.p_sealed = true;
    }
    Ok(())
}

fn finish_spin(a: &mut SpinSettle, seed: Option<[u8; 32]>, now: i64, late: bool) -> Result<()> {
    let owner = a.player.owner;
    let before = owed_of(&a.player);
    // Settlement inputs — before the settlement itself: from them and the round number (RoundRevealed) anyone can recompute the prizes bit for bit.
    if seed.is_some() {
        let i = engine::round_inputs(&a.config, &a.player, now, late);
        emit!(RoundInputs {
            owner,
            commit_slot: a.player.p_commit_slot,
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
        });
    }
    let out = match (seed, late) {
        (Some(v), true) => engine::settle_late(&mut a.config, &mut a.player, v, now)?,
        (s, _) => engine::settle(&mut a.config, &mut a.player, s, now)?,
    };
    track_owed(&mut a.config, before, owed_of(&a.player))?;
    // vault protection: a settlement cannot credit more than the vault holds
    solvent(&a.config, a.vault.amount)?;
    emit!(SpinSettled {
        owner,
        spins: out.spins,
        extras: out.extras,
        gold: out.gold,
        prizes: out.prizes,
        best_prize: out.best_prize,
        mini_hits: out.mini_hits,
        mini_won: out.mini_won,
        jackpot_hits: out.jp_hits,
        jackpot_total: out.jp_pool_total,
        super_hits: out.super_hits,
        super_total: out.super_total,
        forfeited: out.forfeited,
        refunded: false,
    });
    if out.bank_released > 0 {
        emit!(BankReleased { amount: out.bank_released, prize_pool: a.config.prize_pool, seed_reserve: a.config.seed_reserve });
    }
    Ok(())
}

// ---------------- instruction accounts ----------------

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// Only whoever holds the upgrade authority of THIS program can set the launch parameters (protection against front-running).
    /// The address of the program data account is computed from the program id: otherwise one could slip in
    /// a foreign program whose authority belongs to the attacker and take over the launch before the owner.
    #[account(
        constraint = is_own_program_data(&crate::ID, &program_data.key()) @ FdError::Unauthorized,
        constraint = program_data.upgrade_authority_address == Some(payer.key()) @ FdError::Unauthorized
    )]
    pub program_data: Account<'info, ProgramData>,
    #[account(address = crate::ID)]
    /// CHECK: the id of the program itself
    pub program: UncheckedAccount<'info>,
    #[account(init, payer = payer, space = 8 + Config::INIT_SPACE, seeds = [SEED_CONFIG], bump)]
    pub config: Box<Account<'info, Config>>,
    pub mint: Box<Account<'info, Mint>>,
    #[account(init, payer = payer, seeds = [SEED_VAULT], bump, token::mint = mint, token::authority = config)]
    pub vault: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct FundPools<'info> {
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.vault)]
    pub vault: Box<Account<'info, TokenAccount>>,
}

#[derive(Accounts)]
pub struct Register<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(init, payer = user, space = 8 + Player::INIT_SPACE, seeds = [SEED_PLAYER, user.key().as_ref()], bump)]
    pub player: Box<Account<'info, Player>>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Checkin<'info> {
    pub user: Signer<'info>,
    #[account(mut, seeds = [SEED_PLAYER, user.key().as_ref()], bump = player.bump)]
    pub player: Box<Account<'info, Player>>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    pub user: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [SEED_PLAYER, user.key().as_ref()], bump = player.bump)]
    pub player: Box<Account<'info, Player>>,
    #[account(mut, address = config.vault)]
    pub vault: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = config.mint, token::authority = user)]
    pub user_ata: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct DepositSweep<'info> {
    /// Whoever sent the command (the keeper, the player's site or anyone): they are reimbursed the network fee
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: the player's wallet; the deposit address and the player account are derived from it, no signature needed
    #[account(mut)]
    pub owner: UncheckedAccount<'info>,
    /// Personal deposit address: a regular system account at an address derived from the player's wallet
    #[account(mut, seeds = [SEED_DEPOSIT, owner.key().as_ref()], bump)]
    pub deposit: SystemAccount<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    /// Player account: if it does not exist yet, it is created by this same crediting (the sender pays, the cost is reimbursed from the deposit)
    #[account(init_if_needed, payer = payer, space = 8 + Player::INIT_SPACE, seeds = [SEED_PLAYER, owner.key().as_ref()], bump)]
    pub player: Box<Account<'info, Player>>,
    #[account(mut, address = config.vault)]
    pub vault: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Donate<'info> {
    pub user: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, address = config.vault)]
    pub vault: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = config.mint, token::authority = user)]
    pub user_ata: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    pub user: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [SEED_PLAYER, user.key().as_ref()], bump = player.bump)]
    pub player: Box<Account<'info, Player>>,
    #[account(mut, address = config.vault)]
    pub vault: Box<Account<'info, TokenAccount>>,
    #[account(address = config.mint)]
    pub mint: Box<Account<'info, Mint>>,
    #[account(mut, token::mint = config.mint, token::authority = user)]
    pub user_ata: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WithdrawSol<'info> {
    /// Player: signs and receives regular SOL
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [SEED_PLAYER, user.key().as_ref()], bump = player.bump)]
    pub player: Box<Account<'info, Player>>,
    #[account(mut, address = config.vault)]
    pub vault: Box<Account<'info, TokenAccount>>,
    /// The game currency is wrapped SOL (this was already checked at launch, in initialize); here the check is repeated explicitly
    #[account(address = config.mint, constraint = mint.key() == anchor_spl::token::spl_token::native_mint::ID @ FdError::BadParams)]
    pub mint: Box<Account<'info, Mint>>,
    /// Service account of wrapped SOL: the address is derived from the program, the owner is the config account
    #[account(mut, seeds = [SEED_UNWRAP], bump, token::mint = config.mint, token::authority = config)]
    pub unwrap_account: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitUnwrap<'info> {
    /// The administrator: only they can turn `withdraw_sol` on; pays the rent deposit of the service account
    #[account(mut, address = config.dev_authority @ FdError::Unauthorized)]
    pub payer: Signer<'info>,
    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(address = config.mint)]
    pub mint: Box<Account<'info, Mint>>,
    #[account(init, payer = payer, seeds = [SEED_UNWRAP], bump, token::mint = mint, token::authority = config)]
    pub unwrap_account: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SpinCommit<'info> {
    /// Signer: the player's owner or their session key
    #[account(mut)]
    pub user: Signer<'info>,
    /// CHECK: the player's owner; the signer's rights are checked in spin_commit (the owner or an active session)
    pub owner: UncheckedAccount<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [SEED_PLAYER, owner.key().as_ref()], bump = player.bump)]
    pub player: Box<Account<'info, Player>>,
    /// Session: required only when a session key signs
    #[account(mut, seeds = [SEED_SESSION, owner.key().as_ref()], bump = session.bump)]
    pub session: Option<Account<'info, Session>>,
}

#[derive(Accounts)]
pub struct SpinSettle<'info> {
    pub settler: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [SEED_PLAYER, player.owner.as_ref()], bump = player.bump)]
    pub player: Box<Account<'info, Player>>,
    #[account(mut, address = config.vault)]
    pub vault: Box<Account<'info, TokenAccount>>,
    #[account(address = config.mint)]
    pub mint: Box<Account<'info, Mint>>,
    pub token_program: Program<'info, Token>,
    /// CHECK: the system account with the list of recent block hashes; the address is pinned
    #[account(address = SLOT_HASHES_ID)]
    pub slot_hashes: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct SpinSeal<'info> {
    #[account(mut, seeds = [SEED_PLAYER, player.owner.as_ref()], bump = player.bump)]
    pub player: Box<Account<'info, Player>>,
    /// CHECK: the system account with the list of recent block hashes; the address is pinned
    #[account(address = SLOT_HASHES_ID)]
    pub slot_hashes: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct RollEpoch<'info> {
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
}

#[derive(Accounts)]
pub struct ClaimDev<'info> {
    #[account(address = config.dev_authority @ FdError::Unauthorized)]
    pub dev: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, address = config.vault)]
    pub vault: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = config.mint)]
    pub dev_ata: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct SessionOpen<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    /// Only a registered player can open a session
    #[account(seeds = [SEED_PLAYER, user.key().as_ref()], bump = player.bump)]
    pub player: Box<Account<'info, Player>>,
    #[account(init_if_needed, payer = user, space = 8 + Session::INIT_SPACE, seeds = [SEED_SESSION, user.key().as_ref()], bump)]
    pub session: Account<'info, Session>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SessionClose<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut, close = user, seeds = [SEED_SESSION, user.key().as_ref()], bump = session.bump)]
    pub session: Account<'info, Session>,
}

#[derive(Accounts)]
pub struct MiniSeedTopup<'info> {
    #[account(address = config.dev_authority @ FdError::Unauthorized)]
    pub dev: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
}

#[derive(Accounts)]
pub struct SetHouseRoot<'info> {
    /// The root authority if the `RootAuth` account exists, otherwise the holder of the program's upgrade authority
    pub authority: Signer<'info>,
    /// CHECK: the program data account of THIS program. Read and checked in the instruction (address computed from the program
    /// id, owner = the upgradeable loader, recorded authority = the signer) ONLY while the `RootAuth` account does not exist;
    /// once it exists this account is ignored, so nothing here depends on the loader after the program is closed.
    pub program_data: UncheckedAccount<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    /// CHECK: the address is fixed by the seeds, so the caller cannot leave the record out or pass another one; whether it
    /// exists (is owned by this program) and what it says is checked in the instruction.
    #[account(seeds = [SEED_ROOT_AUTH], bump)]
    pub root_auth: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct SetRootAuthority<'info> {
    /// The holder of the program's upgrade authority; pays the rent of the record when it is created
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        constraint = is_own_program_data(&crate::ID, &program_data.key()) @ FdError::Unauthorized,
        constraint = program_data.upgrade_authority_address == Some(authority.key()) @ FdError::Unauthorized
    )]
    pub program_data: Account<'info, ProgramData>,
    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(init_if_needed, payer = authority, space = 8 + RootAuth::INIT_SPACE, seeds = [SEED_ROOT_AUTH], bump)]
    pub root_auth: Account<'info, RootAuth>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ProposeRootAuthority<'info> {
    #[account(address = root_auth.authority @ FdError::NotRootAuthority, constraint = root_authority.key() != Pubkey::default() @ FdError::NotRootAuthority)]
    pub root_authority: Signer<'info>,
    #[account(mut, seeds = [SEED_ROOT_AUTH], bump = root_auth.bump)]
    pub root_auth: Account<'info, RootAuth>,
}

#[derive(Accounts)]
pub struct AcceptRootAuthority<'info> {
    #[account(address = root_auth.pending @ FdError::NotRootAuthority, constraint = new_authority.key() != Pubkey::default() @ FdError::NotRootAuthority)]
    pub new_authority: Signer<'info>,
    #[account(seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [SEED_ROOT_AUTH], bump = root_auth.bump)]
    pub root_auth: Account<'info, RootAuth>,
}

#[derive(Accounts)]
pub struct GuardPause<'info> {
    pub guard: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
}

#[derive(Accounts)]
pub struct AcceptDev<'info> {
    #[account(address = config.pending_dev @ FdError::Unauthorized, constraint = new_dev.key() != Pubkey::default() @ FdError::Unauthorized)]
    pub new_dev: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
}

#[derive(Accounts)]
pub struct PromoPay<'info> {
    #[account(address = config.promo_authority @ FdError::Unauthorized)]
    pub promo: Signer<'info>,
    #[account(mut, seeds = [SEED_CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, address = config.vault)]
    pub vault: Box<Account<'info, TokenAccount>>,
    #[account(mut, token::mint = config.mint)]
    pub dest_ata: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}
