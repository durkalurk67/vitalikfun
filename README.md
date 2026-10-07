# Housepad

A casino launchpad on Solana. Every table plays from one shared SOL bankroll, and coins own shares of that bankroll. Players bet SOL, profit above the payout line is paid out every hour, and each coin's share goes to its buyback wallet.

**This is live on mainnet with real SOL. The code is unaudited.** See [RECOVERY.md](RECOVERY.md) for the owner's fail-safe: pause, withdraw, and closing the program to reclaim its deposit.

## What's in the repo

| File | What it is |
|---|---|
| `index.html` | The whole website in one file: connect a wallet, feed coins, play Plinko, Slots or Crash, run payouts, claim, and the owner tools. |
| `program/lib.rs` | The on-chain program (plain `solana_program`, about 180 KB). Holds the bankroll, mints house shares, takes and settles bets, runs payouts, pays coins, and has the owner fail-safe. |
| `anchor/lib.rs` | Reference copy of the same program written with Anchor. It has the same accounts and instructions, but no fail-safe, and it's about twice the size. It isn't deployed. |
| `RECOVERY.md` | Step-by-step instructions for the owner if anything goes wrong. |

Nothing in this repo is secret. Never commit seed phrases, private keys, wallet export files, or RPC URLs that contain an API key. `.gitignore` blocks `.json` key files.

## How the money moves

1. **A coin buys into the house.** Anyone can feed SOL in a coin's name. The coin's pump.fun creator fees can also be routed to its fee-intake address and swept in. The SOL joins the bankroll, and the coin gets house shares at the current share price.
2. **Players bet SOL at shared tables.** The house only takes a bet if its biggest possible win fits under the per-bet limit, which is 2% of the free bankroll.
3. **Results.** Each bet's result comes from the hash of the first Solana slot after the bet, mixed with the player's own seed and the bet's address. Anyone can settle a bet. A bet that sits unsettled for about 3 minutes counts as a loss.
4. **Payout.** When the payout timer is up, anyone can run the payout. Profit above the payout line is split: 10% to the Housepad treasury and 90% to coins by share. The payout never touches SOL that open bets could still win.
5. **Claim.** Anyone can send a coin's earned profit to that coin's buyback wallet, which buys the coin back and burns it or pays stakers.
6. **Table sponsorship.** Feeds also count toward a race at each table. Scores halve every 12 hours, and the leader's art goes on the felt.

## Game returns

The website and the program compute results the same way. That was checked on 5,000 random rolls.

| Game | Return to player | House edge |
|---|---|---|
| Plinko (any rows or risk) | 98.6% – 99.0% | 1.0% – 1.4% |
| Slots | 97.9% | 2.1% |
| Crash (any cash-out) | 99.0% | 1.0% |

## Owner fail-safe

The owner wallet is written into the program. It's the house admin and the treasury from the moment the house exists. It's also the program's upgrade authority. With it, you can:

- **Pause.** Stops new bets, feeds and fee sweeps. Settling, payouts and claims keep working.
- **Emergency withdraw.** Only while paused. Sends SOL out of the house to any wallet.
- **Change settings.** Payout interval, max win per bet, the Housepad cut, and the treasury wallet.
- **Hand over admin.** Moves the house admin role to another wallet.
- **Upgrade or close the program.** Closing returns its ~0.94 SOL storage deposit.

All of these except handing over admin and changing settings are buttons in **Owner tools** on the site. [RECOVERY.md](RECOVERY.md) walks through each one.

## Hosting the site

**Vercel:** click Add New → Project, import this repo, and click Deploy.

**GitHub Pages:** go to Settings → Pages, pick branch `main` and `/ (root)`, and click Save.

The site connects to Solana through PublicNode's free mainnet RPC by default. For heavier traffic, paste a dedicated RPC URL (Helius, QuickNode, Triton and others have free tiers) under **Network and program** on the site. To make it the default for everyone, set the `CFG` line in `index.html`. If you put a paid URL with an API key in the file, keep the repo private.

## Before this takes serious money

- **Randomness.** Slot hashes are fine at small stakes, but a block producer could sway a result. Move to a verifiable randomness service (Switchboard or ORAO VRF) before raising table limits.
- **Coin registration.** Right now anyone can register any mint. Restrict it to the coin's pump.fun creator, or to launches made through Housepad.
- **Blackjack.** Those tables take feeds, but hands aren't played on-chain yet.
- **A keeper.** Run a small bot that settles bets nobody settled and runs the payout every hour.
- **Owner powers.** The emergency withdraw means coins have to trust the owner. Adding a timelock or a multisig (for example Squads) later makes that easier to trust.
- **An audit.**
- **The law.** Real-money online casinos need a licence in most places, including most of the US. Check where you can legally offer this before promoting it.
