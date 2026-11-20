# FAIRDRUM program

The on-chain program (smart contract) of FAIRDRUM, a game played with SOL on the Solana network. Every bet, balance,
prize and payout is computed by this program; the site is only a front end to it.

| | |
|---|---|
| Program address (main network) | `5ww3w4HrQrrrPcjfxVRvjLpL5vBJqLt7M5ThsnEkkngf` |
| This source | program **version 2**, on the main network since 18 November 2026 |
| Build of this source | 589,880 bytes, SHA-256 `c925ebbabfceac54eab5eaf4513c3a0e74162c2cd59b1e4afdf4cf6667b120bc` |
| Licence | MIT (`LICENSE-MIT`) |

**Which version is on the network.** Version 2, since the upgrade of 18 November 2026 (the table below names the
transaction). Rounds before that day were settled by version 1, whose bytes are NOT a build of this source. Do not take
anyone's word for it, including this file's: compare the bytes yourself (below).

| | |
|---|---|
| Upgrade applied | 18 November 2026 (UTC), slot 471075866, transaction `2MtkK6ED4vA6i47u962FFxX55Bpw9Z1nuvF1jHfiTW2xD3Mjq9JXWqeptRYxTy6URxmx7UCgwuks2ho4msix4wZo` |

## What is here

| Path | What |
|---|---|
| `programs/fairdrum/src` | the program: `lib.rs` (instructions and account checks), `engine.rs` (the rules: bets, settlement, pots, withdrawals), `math.rs` (the draw), `consts.rs` (every number of the game), `state.rs` (accounts and events), `errors.rs` |
| `programs/fairdrum/tests` | 94 tests of the logic, run without a network (`cargo test`) |
| `idl/fairdrum.json` | the interface description (Anchor IDL) of version 2 |
| `Cargo.toml`, `Cargo.lock`, `Anchor.toml`, `.cargo/` | what the build needs; `Cargo.lock` pins every library version |
| `tests/local/fixtures/rich-pot-vectors.json` | a data file one of the tests compares its output with |
| `check-public.sh` | checks that this folder holds nothing but the files above (no keys, no settings, no addresses of servers). The owner's private list of names it also looks for is not part of this repository; without it that one part of the check is skipped and the script says so |

The round checker (is the outcome of a round what the published secret and the block hash give, and are the prizes
what the rules give) is a separate repository: `fairdrum-verify`.

## Tests

```bash
cargo test --locked --release -p fairdrum          # 94 tests
```

One of the tests writes the file `app/test/fixtures/accounts.json` (a fixture for the site's own tests); it is not part
of this repository and can be deleted.

## Build

The file that is deployed is built with **Anchor 1.2.2**. Its `anchor build` runs
`cargo build-sbf --tools-version v1.57 --arch v3`, so either of these gives the same bytes:

```bash
anchor build --no-idl                                                    # anchor-cli 1.2.2
# or, without Anchor:
cargo-build-sbf --tools-version v1.57 --arch v3 --manifest-path programs/fairdrum/Cargo.toml --sbf-out-dir out
sha256sum target/deploy/fairdrum.so        # or out/fairdrum.so
# c925ebbabfceac54eab5eaf4513c3a0e74162c2cd59b1e4afdf4cf6667b120bc   (589,880 bytes)
```

Versions used: anchor-cli 1.2.2; `cargo-build-sbf` of Solana 3.1.10 and of 3.1.13 (both gave the same file); platform
tools v1.57 (downloaded by `cargo-build-sbf` on first use); the libraries exactly as in `Cargo.lock` (anchor-lang 1.2.2).
A bare `cargo-build-sbf` without the two options builds another kind of file (SBPF v0, about 110,000 bytes larger) with
the same behaviour and another hash: it is not the file on the network.

**How far "reproducible" goes.** On 18.11.2026 this source was built seven times, in four ways: `anchor build --no-idl --ignore-keys` in the working tree with the Solana 3.1.13 tools (twice); `cargo-build-sbf --tools-version v1.57 --arch v3` of 3.1.13 directly, in a temporary copy in another folder; the deploy script's own `anchor build` with the Solana 3.1.10 tools (three times); and `cargo-build-sbf` of 3.1.13 from a copy of exactly this folder on one Linux x86-64 machine. All
the files were identical (the hash above). It has NOT been built on another machine or another operating system by
us, and it has NOT been run through `solana-verify` (the Docker-based verifiable build): that is not done yet. If your
build gives another hash with the versions above, please open an issue with the output of `anchor --version`,
`cargo-build-sbf --version` and `sha256sum`.

**About the comments in the source.** Some comments name internal documents that are not part of this repository
(`docs/UPGRADE-2.md`, `docs/CLOSING-RU.md`, `docs/RISKS.md`, `docs/AUDIT.md`, numbered audits). They are the project's
working papers. The comments were left exactly as they are in the source the build was made from: the program's error
messages carry the file name and the line number of the source, so removing a line would change the bytes of the build.
Everything a comment relies on is restated in the comment itself or in `consts.rs`.

## The rules in one place

Every number of the game is in `programs/fairdrum/src/consts.rs`, with compile-time checks of the relations between
them. Per 10,000 of every executed spin: prize bank 8,939, jackpot 88, mini 44, jackpot seed 18, super jackpot 293,
administrator 618 (6.991% of paid bets, because every spin is a free repeat with probability 11.6%). A pot win is
computed from at most 3 starting amounts (2.5 targets for the super jackpot); a fifth of the jackpot amount and a third
of the mini amount stay in the pot for the next winner; the super jackpot keeps 10%. The prize bank keeps 1,600 SOL and
moves what it holds above that to the seed reserve at settlements (at most 0.5% of a bet per spin), from where it
tops up the jackpot and the mini after a win. The administrator can move reserve to the prize bank only while the bank
is below 1,600 SOL (and only up to 1,600 SOL), never into a pot, so the release cannot be undone. No instruction lets
anyone, the administrator included, take the prize bank, the pots or the reserve out. Whatever the pots hold, a bet returns at most 98.393% on average; the long-run
return to players is 93.009% of paid bets once the prize bank holds 1,600 SOL (92.7% before).

**Events are program logs.** Whoever sends a transaction can pad it so that the node cuts its log; the instruction
still succeeds and the events are missing. A settlement whose log was cut cannot be re-computed from its own
transaction. Only the sender of the settlement (the holder of the round's secret) can cause that; no funds move.

## Compare with the program on the network

```bash
solana program dump 5ww3w4HrQrrrPcjfxVRvjLpL5vBJqLt7M5ThsnEkkngf onchain.so -u <a main-network node>
ls -l onchain.so
```

The network stores a program in an account of a fixed size; the file `onchain.so` is that whole account. If the account
is exactly as long as the program (589,880 bytes) the two files are equal and `sha256sum onchain.so` is the hash above.
If the account is longer (it was extended, or an earlier, larger version lived there), `onchain.so` is the program
**followed by zero bytes**, and its hash is different although the program is the same. So compare the beginning and
check that the rest is zeros:

```bash
N=$(stat -c%s target/deploy/fairdrum.so)                       # 589880
head -c "$N" onchain.so | sha256sum                            # must equal: sha256sum target/deploy/fairdrum.so
tail -c +"$((N + 1))" onchain.so | tr -d '\0' | wc -c          # must print 0: nothing but zeros after the program
solana program show 5ww3w4HrQrrrPcjfxVRvjLpL5vBJqLt7M5ThsnEkkngf -u <node>     # "Authority": who can replace the program
```

`Authority: none` means the program can never be replaced again. While an authority is shown, its holder can replace
the program; "Last Deployed In Slot" tells when it was last done.

## Reporting a problem

Open an issue in this repository. Do not post keys, seed phrases or anything from a wallet.
