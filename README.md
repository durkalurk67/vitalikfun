# Housepad

A casino launchpad on Solana. Creators launch coins on pump.fun from Housepad, and every coin gets a stake in one shared casino. Players bet SOL at shared tables. Profit above the payout line is paid out every hour, and each coin's share goes to its buyback wallet.

**This is live on mainnet with real SOL. The code is unaudited.** See [RECOVERY.md](RECOVERY.md) for the owner's fail-safe: pause, withdraw, and closing the program to reclaim its deposit.

## What's in the repo

| File | What it is |
|---|---|
| `public/index.html` | The whole website: launch a coin, play Plinko, Slots or Crash, feed coins, run payouts, claim, and the owner tools. |
| `api/` | A small server (Vercel functions) for launching coins. It uploads the image to pump.fun, then builds the launch transactions. The creator's wallet signs and pays; the server only signs as the brand-new coin's address. Adapted from the Shopi-fi launcher. |
| `package.json`, `vercel.json`, `env.example` | Vercel setup. |
| `program/lib.rs` | The on-chain program (plain `solana_program`, about 190 KB). Holds the bankroll, mints house shares, takes and settles bets, runs payouts, pays coins, and has the owner fail-safe. |
| `anchor/lib.rs` | Reference copy of the same program written with Anchor. It has the same accounts and instructions, but no fail-safe, and it's about twice the size. It isn't deployed. |
| `RECOVERY.md` | Step-by-step instructions for the owner if anything goes wrong. |

Nothing in this repo is secret. Never commit seed phrases, private keys, wallet export files, or RPC URLs that contain an API key. `.gitignore` blocks `.json` key files.

## Launching a coin

On the site, **Launch a coin** takes a name, ticker, image and description, then does everything in two wallet approvals:

1. **Creates the coin on pump.fun** with the creator's wallet as creator. A dev buy is optional.
2. **Registers it with Housepad in the same transaction.** The coin's own address has to sign, and the program checks that the coin really exists on pump.fun, so a coin can only join the casino at its own launch, nobody can squat on someone else's coin, and made-up coins can't get in.
3. **Locks the creator-fee split on pump.fun.** By default it's 10% to the Housepad treasury, 40% to the coin's casino stake, and 50% to the creator. The creator can raise the casino stake, but it can't go below 20%. pump.fun only lets the split be set once, so it's permanent.

Every trade of the coin then pays creator fees, and part of them goes to the coin's casino stake. Anyone can trigger pump.fun's fee payout. Then **Move creator fees into the house** on the coin's card turns that SOL into more house shares.

## How the money moves

1. **A coin buys into the house.** Its share of pump.fun creator fees lands at its casino-stake address and gets moved into the bankroll. Anyone can also feed SOL in a coin's name. Either way the SOL joins the bankroll, and the coin gets house shares at the current share price.
2. **Players bet SOL at shared tables.** The house only takes a bet if its biggest possible win fits under the per-bet limit, which is 2% of the free bankroll. So the biggest bet on a 1,489× Plinko option is tiny until the bankroll is large; the site shows the current limit for each option and says what bankroll an option needs before it opens.
3. **Results.** Each bet's result comes from the hash of the first Solana slot after the bet, mixed with the player's own seed and the bet's address. Anyone can settle a bet. A bet that sits unsettled for about 3 minutes counts as a loss.
4. **Payout.** When the payout timer is up, anyone can run the payout. Profit above the payout line is split: 10% to the Housepad treasury and 90% to coins by share. The payout never touches SOL that open bets could still win.
5. **Claim.** Anyone can send a coin's earned profit to that coin's buyback wallet. What that wallet does with it (buy back and burn, pay holders) is the creator's promise, shown on the coin's card as its buyback plan; the program doesn't enforce it.
6. **Table sponsorship.** Feeds also count toward a race at each table. Scores halve every 12 hours, and the leader's art goes on the felt.

## Game returns

The website and the program compute results the same way. That was checked on 5,000 random rolls.

| Game | Return to player | House edge |
|---|---|---|
| Plinko (any rows or risk) | 98.6% – 99.0% | 1.0% – 1.4% |
| Slots | 97.9% | 2.1% |
| Crash (any cash-out) | 99.0% | 1.0% |

## What's been checked

- The payout only ever pays out profit that no open bet could still win, and a coin can never claim more than the house owes it. A random-play model (400 runs of mixed feeds, bets, payouts, claims and owner withdrawals) never found the books out of balance or a player's stake missing.
- A bet always settles, even after the owner has withdrawn: the win is capped at whatever bankroll is left, and the stake always comes back from escrow first.
- Only real pump.fun coins can join, only at launch, and the house can only be created with its fixed settings.
- The site sends transactions straight from the browser to Solana. There's no relay to abuse, and the server's RPC key is never sent to the page.

## Owner fail-safe

The owner wallet is written into the program. It's the house admin and the treasury from the moment the house exists, and the house is always created with the same fixed settings (hourly payouts, 2% max win, 10% Housepad cut) no matter who pays to create it. The owner wallet is also the program's upgrade authority. With it, you can:

- **Pause.** Stops new bets, feeds and fee sweeps. Settling, payouts and claims keep working.
- **Emergency withdraw.** Only while paused. Sends SOL out of the house to any wallet, from the bankroll first and then unclaimed coin profit. Players' stakes on open bets are never touched, and the house keeps working afterwards.
- **Change settings.** Payout interval, max win per bet, the Housepad cut, and the treasury wallet.
- **Hand over admin.** Moves the house admin role to another wallet.
- **Upgrade or close the program.** Closing returns its ~0.98 SOL storage deposit.

All of these except handing over admin and changing settings are buttons in **Owner tools** on the site. [RECOVERY.md](RECOVERY.md) walks through each one.

## Hosting the site (Vercel)

1. **Put this folder on GitHub.** Create a new repository and upload everything in it.
2. **Import it into Vercel.** Click Add New → Project, pick the repository, and click Deploy. No build settings are needed.
3. **Add environment variables** under Vercel → Project → Settings → Environment Variables, then click Redeploy:
   - `SOLANA_RPC_URL`: your Helius mainnet URL. The same key as Shopi-fi works.
   - `PLATFORM_WALLET`: the wallet that gets Housepad's share of creator fees. It defaults to the owner wallet.
   - `PLATFORM_FEE_BPS`: `1000` for 10%.
   - `HOUSE_MIN_BPS`: `2000` for a 20% minimum casino stake.
   - `HOUSEPAD_PROGRAM_ID`: already set to the deployed program by default.
4. **Connect your domain** under Settings → Domains.

The RPC key stays on the server and is never put in the page. In the browser, the site reads Solana through PublicNode's free mainnet RPC. Anyone can switch that under **Network and program**.

**Without Vercel** (for example GitHub Pages or Netlify Drop, using just `public/index.html`), everything works except launching coins, which needs the `api/` server.

## Test it with a tiny launch first

1. Open the site in a desktop browser with Phantom, and connect.
2. Launch a throwaway coin with no dev buy. You need about 0.03 SOL for fees and deposits.
3. On pump.fun, open the coin and check that the creator-fee split shows the Housepad wallet, the casino stake and your wallet.
4. Make a small buy and sell, then trigger a fee payout on pump.fun. Check that SOL lands in all three, and that **Move creator fees into the house** works on Housepad.

## Before this takes serious money

- **Randomness.** Slot hashes are fine at small stakes, but a block producer could sway a result. Move to a verifiable randomness service (Switchboard or ORAO VRF) before raising table limits.
- **Blackjack.** Those tables take feeds, but hands aren't played on-chain yet.
- **A keeper.** Run a small bot that settles bets nobody settled and runs the payout every hour.
- **Owner powers.** The emergency withdraw means coins have to trust the owner. Adding a timelock or a multisig (for example Squads) later makes that easier to trust.
- **An audit.**
- **The law.** Real-money online casinos need a licence in most places, including most of the US. Check where you can legally offer this before promoting it.
