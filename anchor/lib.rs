// Housepad — one SOL bankroll shared by every table; coins own shares of it.
//
//  * Coins buy house shares with creator fees (sent to their fee-intake address and swept in)
//    or with feeds from anyone. Shares mint at the current share price (NAV).
//  * Players bet SOL at shared tables. Results come from the hash of the first slot after the bet,
//    mixed with the player's seed and the bet address. Unsettled bets expire as losses.
//  * Every payout interval, profit above the high-water mark is paid: platform fee to the treasury,
//    the rest credited to coins by share (accumulator), claimable to each coin's buyback wallet.
//  * Each table keeps a decaying race of feeders; the leader is the table's sponsor (its art).
//
// Devnet build. Unaudited. Not for real funds.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::hash::hashv;
use anchor_lang::solana_program::sysvar;
use anchor_lang::system_program;

declare_id!("11111111111111111111111111111111");

pub const SCALE: u128 = 1_000_000_000_000; // fixed-point scale for share price and accumulator
pub const NUM_TABLES: u8 = 8;
pub const RACE_SLOTS: usize = 8;
pub const HALF_LIFE: i64 = 12 * 3600; // table race scores halve every 12 hours
pub const MAX_CRASH_E2: u64 = 1_000_000; // 10,000x

pub const GAME_PLINKO: u8 = 0;
pub const GAME_SLOTS: u8 = 1;
pub const GAME_CRASH: u8 = 2;

// Plinko multipliers ×100, house edge 1%. Index: rows (8, 12, 16) × risk (low, medium, high).
const PLINKO_8: [[u32; 9]; 3] = [
    [595, 269, 134, 76, 54, 76, 134, 269, 595],
    [1320, 367, 129, 60, 41, 60, 129, 367, 1320],
    [3780, 429, 90, 36, 26, 36, 90, 429, 3780],
];
const PLINKO_12: [[u32; 13]; 3] = [
    [1070, 553, 301, 173, 107, 72, 57, 72, 107, 173, 301, 553, 1070],
    [4480, 1370, 478, 194, 92, 54, 41, 54, 92, 194, 478, 1370, 4480],
    [24200, 3080, 570, 154, 60, 34, 28, 34, 60, 154, 570, 3080, 24200],
];
const PLINKO_16: [[u32; 17]; 3] = [
    [1870, 1030, 592, 351, 217, 140, 96, 70, 58, 70, 96, 140, 217, 351, 592, 1030, 1870],
    [15000, 4810, 1690, 659, 285, 139, 77, 50, 41, 50, 77, 139, 285, 659, 1690, 4810, 15000],
    [148900, 20200, 3580, 829, 250, 99, 51, 34, 30, 34, 51, 99, 250, 829, 3580, 20200, 148900],
];
// Slots: jackpot, seven, bar, gem, star, club, heart. Weights out of 47, three-of-a-kind ×100. Two hearts pay 4.16x.
const SLOT_WEIGHTS: [u64; 7] = [1, 3, 5, 7, 9, 12, 10];
const SLOT_PAY3: [u64; 7] = [41600, 16700, 6250, 2780, 1390, 694, 1110];
const SLOT_TWO_HEARTS: u64 = 416;
const SLOT_HEART: usize = 6;

#[program]
pub mod housepad {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, payout_interval: i64, max_profit_bps: u16, platform_fee_bps: u16) -> Result<()> {
        require!(payout_interval >= 30, HpError::BadParam);
        require!(max_profit_bps > 0 && max_profit_bps <= 1000, HpError::BadParam);
        require!(platform_fee_bps <= 5000, HpError::BadParam);
        let h = &mut ctx.accounts.house;
        h.authority = ctx.accounts.authority.key();
        h.treasury = ctx.accounts.treasury.key();
        h.bump = ctx.bumps.house;
        h.payout_interval = payout_interval;
        h.max_profit_bps = max_profit_bps;
        h.platform_fee_bps = platform_fee_bps;
        h.last_payout = Clock::get()?.unix_timestamp;
        h.hwm_nav = SCALE;
        Ok(())
    }

    pub fn init_table(ctx: Context<InitTable>, id: u8) -> Result<()> {
        require!(id < NUM_TABLES, HpError::BadTable);
        let t = &mut ctx.accounts.table;
        t.id = id;
        t.bump = ctx.bumps.table;
        t.updated = Clock::get()?.unix_timestamp;
        Ok(())
    }

    pub fn set_params(ctx: Context<SetParams>, payout_interval: i64, max_profit_bps: u16, platform_fee_bps: u16) -> Result<()> {
        require!(payout_interval >= 30, HpError::BadParam);
        require!(max_profit_bps > 0 && max_profit_bps <= 1000, HpError::BadParam);
        require!(platform_fee_bps <= 5000, HpError::BadParam);
        let h = &mut ctx.accounts.house;
        h.payout_interval = payout_interval;
        h.max_profit_bps = max_profit_bps;
        h.platform_fee_bps = platform_fee_bps;
        h.treasury = ctx.accounts.treasury.key();
        Ok(())
    }

    pub fn register_coin(ctx: Context<RegisterCoin>, name: String, symbol: String, uri: String, burn_bps: u16, home_table: u8) -> Result<()> {
        require!(name.len() <= 32 && symbol.len() <= 10 && uri.len() <= 200, HpError::TooLong);
        require!(burn_bps <= 10_000, HpError::BadParam);
        require!(home_table < NUM_TABLES, HpError::BadTable);
        let c = &mut ctx.accounts.coin;
        c.mint = ctx.accounts.mint.key();
        c.creator = ctx.accounts.creator.key();
        c.buyback_wallet = ctx.accounts.buyback_wallet.key();
        c.burn_bps = burn_bps;
        c.home_table = home_table;
        c.name = name;
        c.symbol = symbol;
        c.uri = uri;
        c.bump = ctx.bumps.coin;
        c.debt = 0;
        emit!(CoinRegistered { mint: c.mint, creator: c.creator });
        Ok(())
    }

    /// Anyone adds SOL to the house in a coin's name, aimed at one table's race.
    pub fn feed(ctx: Context<Feed>, table_id: u8, amount: u64) -> Result<()> {
        require!(amount > 0, HpError::ZeroAmount);
        system_program::transfer(
            CpiContext::new(ctx.accounts.system_program.to_account_info(), system_program::Transfer {
                from: ctx.accounts.feeder.to_account_info(),
                to: ctx.accounts.house.to_account_info(),
            }),
            amount,
        )?;
        let now = Clock::get()?.unix_timestamp;
        let shares = credit_feed(&mut ctx.accounts.house, &mut ctx.accounts.coin, &mut ctx.accounts.table, amount, now)?;
        emit!(Fed { mint: ctx.accounts.coin.mint, table: table_id, amount, shares: shares as u64, by: ctx.accounts.feeder.key(), sponsor: ctx.accounts.table.sponsor });
        Ok(())
    }

    /// Moves whatever has landed at a coin's fee-intake address (its routed creator fees) into the house.
    pub fn sweep_fees(ctx: Context<SweepFees>) -> Result<()> {
        let amount = ctx.accounts.fee_intake.lamports();
        require!(amount > 0, HpError::ZeroAmount);
        let mint = ctx.accounts.coin.mint;
        let bump = [ctx.bumps.fee_intake];
        let seeds: &[&[u8]] = &[b"fees", mint.as_ref(), &bump];
        system_program::transfer(
            CpiContext::new_with_signer(ctx.accounts.system_program.to_account_info(), system_program::Transfer {
                from: ctx.accounts.fee_intake.to_account_info(),
                to: ctx.accounts.house.to_account_info(),
            }, &[seeds]),
            amount,
        )?;
        let now = Clock::get()?.unix_timestamp;
        let table_id = ctx.accounts.table.id;
        let shares = credit_feed(&mut ctx.accounts.house, &mut ctx.accounts.coin, &mut ctx.accounts.table, amount, now)?;
        emit!(Fed { mint, table: table_id, amount, shares: shares as u64, by: ctx.accounts.fee_intake.key(), sponsor: ctx.accounts.table.sponsor });
        Ok(())
    }

    pub fn place_bet(ctx: Context<PlaceBet>, bet_id: u64, table_id: u8, game: u8, a: u8, b: u8, target_e2: u32, amount: u64, client_seed: [u8; 32]) -> Result<()> {
        let (game_ok, min, max) = table_rules(table_id);
        require!(game_ok == Some(game), HpError::GameNotOnTable);
        require!(amount >= min && amount <= max, HpError::BetOutOfRange);
        let top_e2: u64 = match game {
            GAME_PLINKO => {
                require!(a < 3 && b < 3, HpError::BadParam);
                plinko_row(a, b).iter().copied().max().unwrap_or(0) as u64
            }
            GAME_SLOTS => SLOT_PAY3[0],
            GAME_CRASH => {
                require!(target_e2 >= 101 && (target_e2 as u64) <= MAX_CRASH_E2, HpError::BadParam);
                target_e2 as u64
            }
            _ => return err!(HpError::GameNotOnTable),
        };
        let max_payout = (amount as u128 * top_e2 as u128 / 100) as u64;
        let h = &mut ctx.accounts.house;
        let free = h.bankroll.saturating_sub(h.liability);
        let profit_cap = (free as u128 * h.max_profit_bps as u128 / 10_000) as u64;
        require!(max_payout.saturating_sub(amount) <= profit_cap, HpError::OverMaxWin);

        system_program::transfer(
            CpiContext::new(ctx.accounts.system_program.to_account_info(), system_program::Transfer {
                from: ctx.accounts.player.to_account_info(),
                to: h.to_account_info(),
            }),
            amount,
        )?;
        h.escrow = h.escrow.checked_add(amount).ok_or(HpError::Math)?;
        h.liability = h.liability.checked_add(max_payout.saturating_sub(amount)).ok_or(HpError::Math)?;
        h.wagered = h.wagered.saturating_add(amount);
        h.bets = h.bets.saturating_add(1);
        let t = &mut ctx.accounts.table;
        t.wagered = t.wagered.saturating_add(amount);
        t.bets = t.bets.saturating_add(1);

        let bet = &mut ctx.accounts.bet;
        bet.player = ctx.accounts.player.key();
        bet.id = bet_id;
        bet.table_id = table_id;
        bet.game = game;
        bet.a = a;
        bet.b = b;
        bet.target_e2 = target_e2;
        bet.amount = amount;
        bet.max_payout = max_payout;
        bet.slot = Clock::get()?.slot;
        bet.client_seed = client_seed;
        bet.bump = ctx.bumps.bet;
        emit!(BetPlaced { player: bet.player, bet: bet.key(), table: table_id, game, amount, slot: bet.slot });
        Ok(())
    }

    /// Anyone can settle. The result uses the hash of the first slot after the bet's slot.
    /// If that hash has rolled out of the SlotHashes window, the bet expired and counts as a loss.
    pub fn settle_bet(ctx: Context<SettleBet>) -> Result<()> {
        let bet = &ctx.accounts.bet;
        let current = Clock::get()?.slot;
        require!(current > bet.slot + 1, HpError::TooEarly);
        let data = ctx.accounts.slot_hashes.try_borrow_data()?;
        require!(data.len() >= 8, HpError::BadSysvar);
        let n = u64::from_le_bytes(data[0..8].try_into().unwrap()) as usize;
        let mut found: Option<[u8; 32]> = None;
        let mut expired = true;
        for i in 0..n {
            let off = 8 + i * 40;
            if off + 40 > data.len() { break; }
            let s = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
            if s > bet.slot {
                let mut hsh = [0u8; 32];
                hsh.copy_from_slice(&data[off + 8..off + 40]);
                found = Some(hsh);
            } else {
                expired = false;
                break;
            }
        }
        drop(data);
        let (payout, roll) = match (found, expired) {
            (Some(sh), false) => {
                let r = hashv(&[&sh, &bet.client_seed, bet.key().as_ref()]).to_bytes();
                (outcome(bet, &r), r)
            }
            (None, false) => return err!(HpError::TooEarly),
            _ => (0u64, [0u8; 32]),
        };

        let amount = bet.amount;
        let h = &mut ctx.accounts.house;
        h.escrow = h.escrow.checked_sub(amount).ok_or(HpError::Math)?;
        h.liability = h.liability.saturating_sub(bet.max_payout.saturating_sub(amount));
        // escrowed stake joins the bankroll, then the payout leaves it
        h.bankroll = h.bankroll.checked_add(amount).ok_or(HpError::Math)?;
        h.bankroll = h.bankroll.checked_sub(payout).ok_or(HpError::Math)?;
        if payout > 0 {
            move_lamports(&h.to_account_info(), &ctx.accounts.player.to_account_info(), payout)?;
        }
        emit!(BetSettled { player: bet.player, bet: bet.key(), table: bet.table_id, game: bet.game, amount, payout, expired, roll });
        Ok(())
    }

    /// Permissionless. Pays profit above the high-water mark: platform fee out, the rest to coins by share.
    pub fn crank_payout(ctx: Context<CrankPayout>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let h = &mut ctx.accounts.house;
        require!(now >= h.last_payout + h.payout_interval, HpError::TooEarly);
        h.last_payout = now;
        if h.total_shares == 0 { return Ok(()); }
        let nav = h.bankroll as u128 * SCALE / h.total_shares;
        if nav <= h.hwm_nav {
            emit!(PayoutSkipped { below: ((h.hwm_nav - nav) * h.total_shares / SCALE) as u64 });
            return Ok(());
        }
        let mut profit = ((nav - h.hwm_nav) * h.total_shares / SCALE) as u64;
        // never pay out SOL that open bets could still win; the rest waits for the next payout
        profit = profit.min(h.bankroll.saturating_sub(h.liability));
        if profit == 0 { return Ok(()); }
        let fee = (profit as u128 * h.platform_fee_bps as u128 / 10_000) as u64;
        let to_coins = profit - fee;
        h.bankroll = h.bankroll.checked_sub(profit).ok_or(HpError::Math)?;
        h.owed = h.owed.checked_add(to_coins).ok_or(HpError::Math)?;
        h.acc_per_share = h.acc_per_share.checked_add(to_coins as u128 * SCALE / h.total_shares).ok_or(HpError::Math)?;
        h.paid_to_coins = h.paid_to_coins.saturating_add(to_coins);
        h.paid_to_platform = h.paid_to_platform.saturating_add(fee);
        if fee > 0 {
            move_lamports(&h.to_account_info(), &ctx.accounts.treasury.to_account_info(), fee)?;
        }
        emit!(Payout { profit, platform: fee, to_coins, nav: nav as u64 });
        Ok(())
    }

    /// Permissionless. Sends a coin's earned profit to its buyback wallet, which buys the coin
    /// on the market and burns it or pays stakers on the coin's split.
    pub fn claim_coin(ctx: Context<ClaimCoin>) -> Result<()> {
        let h = &mut ctx.accounts.house;
        let c = &mut ctx.accounts.coin;
        accrue(h, c);
        // capped at what the house holds for coins, so per-share rounding can never pull from the bankroll
        let amount = c.claimable.min(h.owed);
        require!(amount > 0, HpError::ZeroAmount);
        c.claimable -= amount;
        c.claimed_total = c.claimed_total.saturating_add(amount);
        h.owed -= amount;
        move_lamports(&h.to_account_info(), &ctx.accounts.buyback_wallet.to_account_info(), amount)?;
        emit!(Claimed { mint: c.mint, amount, to: c.buyback_wallet });
        Ok(())
    }
}

/* ---------------- helpers ---------------- */

fn table_rules(id: u8) -> (Option<u8>, u64, u64) {
    const SOL: u64 = 1_000_000_000;
    match id {
        0 => (Some(GAME_PLINKO), SOL / 1000, SOL),
        1 => (Some(GAME_PLINKO), SOL / 20, 10 * SOL),
        2 => (Some(GAME_SLOTS), SOL / 1000, SOL / 2),
        3 => (Some(GAME_SLOTS), SOL / 20, 5 * SOL),
        7 => (Some(GAME_CRASH), SOL / 1000, 20 * SOL),
        _ => (None, 0, 0), // blackjack tables take feeds; their hands are not on-chain yet
    }
}

fn plinko_row(rows_idx: u8, risk: u8) -> &'static [u32] {
    match rows_idx {
        0 => &PLINKO_8[risk as usize],
        1 => &PLINKO_12[risk as usize],
        _ => &PLINKO_16[risk as usize],
    }
}

fn slot_symbol(x: u32) -> usize {
    let v = (x as u64 * 47) >> 32;
    let mut acc = 0u64;
    for (i, w) in SLOT_WEIGHTS.iter().enumerate() {
        acc += w;
        if v < acc { return i; }
    }
    SLOT_WEIGHTS.len() - 1
}

/// Payout in lamports for a settled bet given its 32 random bytes.
fn outcome(bet: &Bet, r: &[u8; 32]) -> u64 {
    let amt = bet.amount as u128;
    match bet.game {
        GAME_PLINKO => {
            let rows: usize = match bet.a { 0 => 8, 1 => 12, _ => 16 };
            let mut k = 0usize;
            for i in 0..rows {
                if (r[i / 8] >> (i % 8)) & 1 == 1 { k += 1; }
            }
            let m = plinko_row(bet.a, bet.b)[k] as u128;
            (amt * m / 100) as u64
        }
        GAME_SLOTS => {
            let s: Vec<usize> = (0..3).map(|i| slot_symbol(u32::from_le_bytes(r[i * 4..i * 4 + 4].try_into().unwrap()))).collect();
            let m: u64 = if s[0] == s[1] && s[1] == s[2] {
                SLOT_PAY3[s[0]]
            } else if s.iter().filter(|&&x| x == SLOT_HEART).count() == 2 {
                SLOT_TWO_HEARTS
            } else { 0 };
            (amt * m as u128 / 100) as u64
        }
        GAME_CRASH => {
            let crash = crash_e2(r);
            if crash >= bet.target_e2 as u64 { (amt * bet.target_e2 as u128 / 100) as u64 } else { 0 }
        }
        _ => 0,
    }
}

/// Crash point ×100 with a 1% edge: 99 / (1 − r), floored, capped at 10,000x.
pub fn crash_e2(r: &[u8; 32]) -> u64 {
    let x = u64::from_le_bytes(r[0..8].try_into().unwrap()) as u128;
    let two64: u128 = 1u128 << 64;
    let v = 99u128 * two64 / (two64 - x);
    (v.min(MAX_CRASH_E2 as u128) as u64).max(100)
}

fn move_lamports(from: &AccountInfo, to: &AccountInfo, amount: u64) -> Result<()> {
    let from_new = from.lamports().checked_sub(amount).ok_or(HpError::Math)?;
    let to_new = to.lamports().checked_add(amount).ok_or(HpError::Math)?;
    **from.try_borrow_mut_lamports()? = from_new;
    **to.try_borrow_mut_lamports()? = to_new;
    Ok(())
}

fn accrue(h: &House, c: &mut Coin) {
    let accrued = c.shares * h.acc_per_share / SCALE;
    let pending = accrued.saturating_sub(c.debt);
    c.claimable = c.claimable.saturating_add(pending as u64);
    c.debt = accrued;
}

fn decay(score: u64, dt: i64) -> u64 {
    if dt <= 0 || score == 0 { return score; }
    let halvings = dt / HALF_LIFE;
    if halvings >= 64 { return 0; }
    let s = (score >> halvings) as u128;
    let rem = (dt % HALF_LIFE) as u128;
    (s - s * rem / (2 * HALF_LIFE as u128)) as u64
}

/// Mints shares for a coin at the current share price and updates the table race.
fn credit_feed(h: &mut House, c: &mut Coin, t: &mut Table, amount: u64, now: i64) -> Result<u128> {
    accrue(h, c);
    let nav = if h.total_shares == 0 { h.hwm_nav } else { h.bankroll as u128 * SCALE / h.total_shares };
    require!(nav > 0, HpError::Math);
    let shares = amount as u128 * SCALE / nav;
    h.bankroll = h.bankroll.checked_add(amount).ok_or(HpError::Math)?;
    h.total_shares = h.total_shares.checked_add(shares).ok_or(HpError::Math)?;
    h.fed = h.fed.saturating_add(amount);
    c.shares = c.shares.checked_add(shares).ok_or(HpError::Math)?;
    c.debt = c.shares * h.acc_per_share / SCALE;
    c.fed_total = c.fed_total.saturating_add(amount);

    // race: decay everyone to now, add to this coin (or take the weakest slot), pick the leader
    let dt = now - t.updated;
    for e in t.race.iter_mut() { e.score = decay(e.score, dt); }
    t.updated = now;
    let key = c.mint;
    let mut idx = t.race.iter().position(|e| e.coin == key && e.score > 0);
    if idx.is_none() {
        let (min_i, min_e) = t.race.iter().enumerate().min_by_key(|(_, e)| e.score).unwrap();
        if min_e.score < amount { idx = Some(min_i); t.race[min_i] = RaceEntry { coin: key, score: 0 }; }
    }
    if let Some(i) = idx { t.race[i].score = t.race[i].score.saturating_add(amount); }
    let cur = t.race.iter().find(|e| e.coin == t.sponsor && e.score > 0).map(|e| e.score).unwrap_or(0);
    let (_, best) = t.race.iter().enumerate().max_by_key(|(_, e)| e.score).unwrap();
    if best.score > cur { t.sponsor = best.coin; }
    Ok(shares)
}

/* ---------------- accounts ---------------- */

#[account]
#[derive(InitSpace)]
pub struct House {
    pub authority: Pubkey,
    pub treasury: Pubkey,
    pub bankroll: u64,        // settled SOL behind the tables
    pub escrow: u64,          // stakes of unsettled bets
    pub liability: u64,       // most the unsettled bets could still take from the bankroll
    pub owed: u64,            // profit credited to coins, not yet claimed
    pub total_shares: u128,
    pub acc_per_share: u128,  // lamports per share, ×SCALE
    pub hwm_nav: u128,        // payout line: lamports per share, ×SCALE
    pub last_payout: i64,
    pub payout_interval: i64,
    pub max_profit_bps: u16,
    pub platform_fee_bps: u16,
    pub wagered: u64,
    pub bets: u64,
    pub fed: u64,
    pub paid_to_coins: u64,
    pub paid_to_platform: u64,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default, InitSpace)]
pub struct RaceEntry {
    pub coin: Pubkey,
    pub score: u64,
}

#[account]
#[derive(InitSpace)]
pub struct Table {
    pub id: u8,
    pub bump: u8,
    pub sponsor: Pubkey,
    pub updated: i64,
    pub wagered: u64,
    pub bets: u64,
    pub race: [RaceEntry; 8],
}

#[account]
#[derive(InitSpace)]
pub struct Coin {
    pub mint: Pubkey,
    pub creator: Pubkey,
    pub buyback_wallet: Pubkey,
    pub burn_bps: u16,
    pub home_table: u8,
    pub shares: u128,
    pub debt: u128,
    pub claimable: u64,
    pub fed_total: u64,
    pub claimed_total: u64,
    #[max_len(32)]
    pub name: String,
    #[max_len(10)]
    pub symbol: String,
    #[max_len(200)]
    pub uri: String,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Bet {
    pub player: Pubkey,
    pub id: u64,
    pub table_id: u8,
    pub game: u8,
    pub a: u8,
    pub b: u8,
    pub target_e2: u32,
    pub amount: u64,
    pub max_payout: u64,
    pub slot: u64,
    pub client_seed: [u8; 32],
    pub bump: u8,
}

/* ---------------- contexts ---------------- */

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = authority, space = 8 + House::INIT_SPACE, seeds = [b"house"], bump)]
    pub house: Account<'info, House>,
    #[account(mut)]
    pub authority: Signer<'info>,
    /// CHECK: wallet that receives the platform fee
    pub treasury: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(id: u8)]
pub struct InitTable<'info> {
    #[account(seeds = [b"house"], bump = house.bump, has_one = authority)]
    pub house: Account<'info, House>,
    #[account(init, payer = authority, space = 8 + Table::INIT_SPACE, seeds = [b"table", id.to_le_bytes().as_ref()], bump)]
    pub table: Account<'info, Table>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetParams<'info> {
    #[account(mut, seeds = [b"house"], bump = house.bump, has_one = authority)]
    pub house: Account<'info, House>,
    pub authority: Signer<'info>,
    /// CHECK: new platform fee wallet
    pub treasury: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct RegisterCoin<'info> {
    #[account(seeds = [b"house"], bump = house.bump)]
    pub house: Account<'info, House>,
    #[account(init, payer = creator, space = 8 + Coin::INIT_SPACE, seeds = [b"coin", mint.key().as_ref()], bump)]
    pub coin: Account<'info, Coin>,
    /// CHECK: the coin's mint address; recorded, not read
    pub mint: UncheckedAccount<'info>,
    /// CHECK: wallet that receives this coin's profit share to buy the coin back
    pub buyback_wallet: UncheckedAccount<'info>,
    #[account(mut)]
    pub creator: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(table_id: u8)]
pub struct Feed<'info> {
    #[account(mut, seeds = [b"house"], bump = house.bump)]
    pub house: Account<'info, House>,
    #[account(mut, seeds = [b"coin", coin.mint.as_ref()], bump = coin.bump)]
    pub coin: Account<'info, Coin>,
    #[account(mut, seeds = [b"table", table_id.to_le_bytes().as_ref()], bump = table.bump)]
    pub table: Account<'info, Table>,
    #[account(mut)]
    pub feeder: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SweepFees<'info> {
    #[account(mut, seeds = [b"house"], bump = house.bump)]
    pub house: Account<'info, House>,
    #[account(mut, seeds = [b"coin", coin.mint.as_ref()], bump = coin.bump)]
    pub coin: Account<'info, Coin>,
    #[account(mut, seeds = [b"table", coin.home_table.to_le_bytes().as_ref()], bump = table.bump)]
    pub table: Account<'info, Table>,
    #[account(mut, seeds = [b"fees", coin.mint.as_ref()], bump)]
    pub fee_intake: SystemAccount<'info>,
    pub cranker: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(bet_id: u64, table_id: u8)]
pub struct PlaceBet<'info> {
    #[account(mut, seeds = [b"house"], bump = house.bump)]
    pub house: Account<'info, House>,
    #[account(mut, seeds = [b"table", table_id.to_le_bytes().as_ref()], bump = table.bump)]
    pub table: Account<'info, Table>,
    #[account(init, payer = player, space = 8 + Bet::INIT_SPACE, seeds = [b"bet", player.key().as_ref(), bet_id.to_le_bytes().as_ref()], bump)]
    pub bet: Account<'info, Bet>,
    #[account(mut)]
    pub player: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SettleBet<'info> {
    #[account(mut, seeds = [b"house"], bump = house.bump)]
    pub house: Account<'info, House>,
    #[account(mut, close = player, has_one = player)]
    pub bet: Account<'info, Bet>,
    /// CHECK: the bettor; receives the payout and the bet account's rent (checked by has_one)
    #[account(mut)]
    pub player: UncheckedAccount<'info>,
    /// CHECK: SlotHashes sysvar, read raw
    #[account(address = sysvar::slot_hashes::ID)]
    pub slot_hashes: UncheckedAccount<'info>,
    pub cranker: Signer<'info>,
}

#[derive(Accounts)]
pub struct CrankPayout<'info> {
    #[account(mut, seeds = [b"house"], bump = house.bump, has_one = treasury)]
    pub house: Account<'info, House>,
    /// CHECK: platform fee wallet (checked by has_one)
    #[account(mut)]
    pub treasury: UncheckedAccount<'info>,
    pub cranker: Signer<'info>,
}

#[derive(Accounts)]
pub struct ClaimCoin<'info> {
    #[account(mut, seeds = [b"house"], bump = house.bump)]
    pub house: Account<'info, House>,
    #[account(mut, seeds = [b"coin", coin.mint.as_ref()], bump = coin.bump, has_one = buyback_wallet)]
    pub coin: Account<'info, Coin>,
    /// CHECK: the coin's buyback wallet (checked by has_one)
    #[account(mut)]
    pub buyback_wallet: UncheckedAccount<'info>,
    pub cranker: Signer<'info>,
}

/* ---------------- events & errors ---------------- */

#[event]
pub struct CoinRegistered { pub mint: Pubkey, pub creator: Pubkey }
#[event]
pub struct Fed { pub mint: Pubkey, pub table: u8, pub amount: u64, pub shares: u64, pub by: Pubkey, pub sponsor: Pubkey }
#[event]
pub struct BetPlaced { pub player: Pubkey, pub bet: Pubkey, pub table: u8, pub game: u8, pub amount: u64, pub slot: u64 }
#[event]
pub struct BetSettled { pub player: Pubkey, pub bet: Pubkey, pub table: u8, pub game: u8, pub amount: u64, pub payout: u64, pub expired: bool, pub roll: [u8; 32] }
#[event]
pub struct Payout { pub profit: u64, pub platform: u64, pub to_coins: u64, pub nav: u64 }
#[event]
pub struct PayoutSkipped { pub below: u64 }
#[event]
pub struct Claimed { pub mint: Pubkey, pub amount: u64, pub to: Pubkey }

#[error_code]
pub enum HpError {
    #[msg("Parameter out of range")]
    BadParam,
    #[msg("No such table")]
    BadTable,
    #[msg("Text is too long")]
    TooLong,
    #[msg("Amount must be above zero")]
    ZeroAmount,
    #[msg("That game is not played on-chain at this table")]
    GameNotOnTable,
    #[msg("Bet is outside this table's limits")]
    BetOutOfRange,
    #[msg("A win this size would pay more than the house allows per bet")]
    OverMaxWin,
    #[msg("Too early")]
    TooEarly,
    #[msg("Could not read slot hashes")]
    BadSysvar,
    #[msg("Arithmetic overflow")]
    Math,
}
