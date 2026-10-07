# Housepad: how to recover your SOL

Your wallet controls the casino in two ways:

- **House admin.** This is built into the program. You can pause the casino and pull SOL out of it.
- **Program owner** (Solana calls this the *upgrade authority*). You can upgrade the program, or close it and get its storage deposit back (about 0.94 SOL).

Nobody else can do either. Keep your wallet's seed phrase safe, because it's the only key to all of this.

## The quick way: Owner tools on the site

1. Open the Housepad site and click **Connect wallet** with your owner wallet. An **Owner tools** panel appears at the top. Nobody else sees it.
2. **Pause.** Click **Pause**. New bets, feeds and creator-fee sweeps stop. Bets already placed can still settle, and payouts and coin claims keep working. Click **Resume** to reopen.
3. **Withdraw.** While paused, leave the amount empty and click **Withdraw** to send everything in the house to your wallet. You can also type an amount. The SOL comes out of the bankroll first, then any coin profit not yet claimed, then the stakes of bets that haven't settled.
   - **Withdrawing everything shuts the house for good.** Once the bankroll is empty, it can't take feeds or bets again, even after you resume. To reopen after that, you'd upload a new version of the program. A partial withdrawal leaves the house usable.
4. **Close the program.** This is only needed if you're shutting down for good. Once the house is empty, click **Close program** and confirm. The program's deposit, about 0.94 SOL, goes back to your wallet. This can't be undone, and that program address can never be used again.

Each step is one transaction that you approve in your wallet. It costs a fraction of a cent.

**Never close the program while there's SOL in the house.** If you do, that SOL is locked forever. The button stays disabled until the house is empty, to stop that from happening.

## If the site is down or gone

The site is a single file, `index.html`. Your SOL lives on Solana, not on the site, so all you need to do is put the file back online:

- **Vercel:** create a project from the GitHub repo and click Deploy, the same way as before.
- **GitHub Pages** (free, no other account needed): in the repo, go to **Settings → Pages**. Under *Branch*, pick `main` and `/ (root)` and click Save. After a minute or two the site is at `https://<your-username>.github.io/<repo-name>/`. This needs `index.html` at the top level of the repo.
- **Netlify Drop:** drag the folder holding `index.html` onto app.netlify.com/drop.

Then follow the steps above. Wallet extensions don't work on a file opened straight from your computer, so it has to be on a web address.

If the default network connection is slow or refuses requests, open **Network and program** on the site, paste another Solana mainnet RPC URL (for example a free one from Helius or QuickNode), and click **Save and reload**.

## If something looks wrong

- **The house shows *Paused* and you didn't pause it.** That can't happen. Only your wallet can pause it. Check that you connected the right wallet.
- **A bet won't settle.** Anyone can settle a bet, and the site's *Your open bets* list has a Settle button. A bet that sits unsettled for about 3 minutes counts as a loss when it's finally settled.
- **The program has a bug.** Pause first, then withdraw. As the program owner you can also upload a fixed version later.

## Addresses

| What | Address |
|---|---|
| Program | `9HxeupB5dzsGzG9xzrohMpPYeyNjbXHKCHVVRTBtEYQY` |
| Owner wallet (admin, program owner, treasury) | `CBdVbUjVuhMSDH39DoYrDg3TCqXFa8cWQrm68chBmvb8` |
| Deploy wallet (only used to upload the program; holds a few cents of leftover SOL) | `3esxTbGTzZ7P4ucbrRvVkMGH4xPPnVwt7GywdiMWXZHP` |

You can look any of these up on explorer.solana.com.
