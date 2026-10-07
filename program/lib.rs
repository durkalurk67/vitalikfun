// Housepad (lite build) — one SOL bankroll shared by every table; coins own shares of it.
//
// Same accounts, instructions, discriminators, byte layouts and events as the Anchor build,
// written against solana_program directly so the deployed binary (and its rent) is much smaller.
// The same website talks to either build.
//
// Fail-safe: the owner wallet (ADMIN below) is the house admin from the start. It can pause new bets
// and feeds, withdraw everything from a paused house, change parameters, and hand admin to another wallet.
//
// Unaudited.

use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint,
    entrypoint::ProgramResult,
    hash::hashv,
    log::{sol_log, sol_log_data},
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_instruction, system_program,
    sysvar::{slot_hashes, Sysvar},
};

entrypoint!(process_instruction);

/// The owner's wallet: house admin and Housepad treasury from the moment the house is created.
// CBdVbUjVuhMSDH39DoYrDg3TCqXFa8cWQrm68chBmvb8
const ADMIN_BYTES: [u8; 32] = [166, 41, 160, 134, 191, 8, 2, 105, 6, 13, 166, 155, 79, 57, 249, 219, 149, 38, 21, 59, 54, 17, 210, 206, 10, 184, 214, 105, 249, 231, 207, 239];
fn admin() -> Pubkey { Pubkey::new_from_array(ADMIN_BYTES) }

/// pump.fun's program. A coin can only join the casino if its pump.fun bonding curve exists.
const PUMP_PROGRAM: Pubkey = Pubkey::new_from_array([1, 86, 224, 246, 147, 102, 90, 207, 68, 219, 21, 104, 191, 23, 91, 170, 81, 137, 203, 151, 245, 210, 255, 59, 101, 93, 43, 182, 253, 109, 24, 176]);
/// House settings on day one. The owner can change them later with set_params.
const DEFAULT_PAYOUT_INTERVAL: i64 = 3600;
const DEFAULT_MAX_PROFIT_BPS: u16 = 200;
const DEFAULT_PLATFORM_FEE_BPS: u16 = 1000;

const SCALE: u128 = 1_000_000_000_000;
const NUM_TABLES: u8 = 8;
const HALF_LIFE: i64 = 12 * 3600;
const MAX_CRASH_E2: u64 = 1_000_000;
const GAME_PLINKO: u8 = 0;
const GAME_SLOTS: u8 = 1;
const GAME_CRASH: u8 = 2;

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
const SLOT_WEIGHTS: [u64; 7] = [1, 3, 5, 7, 9, 12, 10];
const SLOT_PAY3: [u64; 7] = [41600, 16700, 6250, 2780, 1390, 694, 1110];
const SLOT_TWO_HEARTS: u64 = 416;
const SLOT_HEART: usize = 6;

// Anchor-compatible discriminators: sha256("global:<ix>"), sha256("account:<Name>"), sha256("event:<Name>")
const IX_INITIALIZE: [u8; 8] = [175, 175, 109, 31, 13, 152, 155, 237];
const IX_INIT_TABLE: [u8; 8] = [236, 14, 14, 155, 160, 169, 133, 111];
const IX_SET_PARAMS: [u8; 8] = [27, 234, 178, 52, 147, 2, 187, 141];
const IX_REGISTER_COIN: [u8; 8] = [79, 37, 188, 46, 209, 104, 90, 10];
const IX_FEED: [u8; 8] = [46, 213, 237, 176, 190, 113, 182, 94];
const IX_SWEEP_FEES: [u8; 8] = [175, 225, 98, 71, 118, 66, 34, 148];
const IX_PLACE_BET: [u8; 8] = [222, 62, 67, 220, 63, 166, 126, 33];
const IX_SETTLE_BET: [u8; 8] = [115, 55, 234, 177, 227, 4, 10, 67];
const IX_CRANK_PAYOUT: [u8; 8] = [130, 179, 98, 243, 134, 248, 201, 81];
const IX_CLAIM_COIN: [u8; 8] = [222, 156, 154, 212, 188, 143, 84, 105];
const IX_SET_PAUSED: [u8; 8] = [91, 60, 125, 192, 176, 225, 166, 218];
const IX_EMERGENCY_WITHDRAW: [u8; 8] = [239, 45, 203, 64, 150, 73, 218, 92];
const IX_SET_AUTHORITY: [u8; 8] = [133, 250, 37, 21, 110, 163, 26, 121];
const ACC_HOUSE: [u8; 8] = [21, 145, 94, 109, 254, 199, 210, 151];
const ACC_TABLE: [u8; 8] = [34, 100, 138, 97, 236, 129, 230, 112];
const ACC_COIN: [u8; 8] = [215, 195, 53, 238, 217, 196, 213, 51];
const ACC_BET: [u8; 8] = [147, 23, 35, 59, 15, 75, 155, 32];
const EV_COIN_REGISTERED: [u8; 8] = [73, 189, 19, 88, 236, 80, 205, 170];
const EV_FED: [u8; 8] = [123, 143, 103, 16, 214, 190, 110, 102];
const EV_BET_PLACED: [u8; 8] = [88, 88, 145, 226, 126, 206, 32, 0];
const EV_BET_SETTLED: [u8; 8] = [57, 145, 224, 160, 62, 119, 227, 206];
const EV_PAYOUT: [u8; 8] = [156, 247, 34, 25, 6, 177, 75, 15];
const EV_PAYOUT_SKIPPED: [u8; 8] = [42, 46, 147, 182, 4, 134, 82, 74];
const EV_CLAIMED: [u8; 8] = [217, 192, 123, 72, 108, 150, 248, 33];
const EV_PAUSED: [u8; 8] = [172, 248, 5, 253, 49, 255, 255, 232];
const EV_EMERGENCY_WITHDRAW: [u8; 8] = [128, 80, 236, 119, 137, 129, 241, 144];
const EV_AUTHORITY_CHANGED: [u8; 8] = [31, 19, 174, 152, 4, 82, 215, 226];

// Account sizes, identical to the Anchor build (8-byte discriminator + fields)
const HOUSE_SPACE: usize = 8 + 206; // Anchor layout + a trailing `paused` byte
const TABLE_SPACE: usize = 8 + 378;
const COIN_SPACE: usize = 8 + 413; // symbol up to 13 characters, like pump.fun
const BET_SPACE: usize = 8 + 105;

/* ---------------- errors (Anchor-style codes and messages) ---------------- */

#[derive(Clone, Copy)]
enum HpError { BadParam = 0, BadTable, TooLong, ZeroAmount, GameNotOnTable, BetOutOfRange, OverMaxWin, TooEarly, BadSysvar, Math, BadAccount, Paused, NotOwner, NotPaused, NotPumpCoin }

fn fail(e: HpError) -> ProgramError {
    sol_log(match e {
        HpError::BadParam => "Error Message: Parameter out of range",
        HpError::BadTable => "Error Message: No such table",
        HpError::TooLong => "Error Message: Text is too long",
        HpError::ZeroAmount => "Error Message: Amount must be above zero",
        HpError::GameNotOnTable => "Error Message: That game is not played on-chain at this table",
        HpError::BetOutOfRange => "Error Message: Bet is outside this table's limits",
        HpError::OverMaxWin => "Error Message: A win this size would pay more than the house allows per bet",
        HpError::TooEarly => "Error Message: Too early",
        HpError::BadSysvar => "Error Message: Could not read slot hashes",
        HpError::Math => "Error Message: Arithmetic overflow",
        HpError::BadAccount => "Error Message: Wrong account for this instruction",
        HpError::Paused => "Error Message: The house is paused",
        HpError::NotOwner => "Error Message: Only the house owner can do this",
        HpError::NotPaused => "Error Message: Pause the house before withdrawing",
        HpError::NotPumpCoin => "Error Message: Only coins launched on pump.fun can join the casino",
    });
    ProgramError::Custom(6000 + e as u32)
}
fn math() -> ProgramError { fail(HpError::Math) }
macro_rules! require {
    ($c:expr, $e:expr) => { if !($c) { return Err(fail($e)); } };
}

/* ---------------- byte reading and writing ---------------- */

struct Rd<'a> { b: &'a [u8], o: usize }
impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ProgramError> {
        if self.o + n > self.b.len() { return Err(ProgramError::InvalidInstructionData); }
        let s = &self.b[self.o..self.o + n];
        self.o += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, ProgramError> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16, ProgramError> { Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> Result<u32, ProgramError> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> Result<u64, ProgramError> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn i64(&mut self) -> Result<i64, ProgramError> { Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn u128(&mut self) -> Result<u128, ProgramError> { Ok(u128::from_le_bytes(self.take(16)?.try_into().unwrap())) }
    fn arr32(&mut self) -> Result<[u8; 32], ProgramError> { Ok(self.take(32)?.try_into().unwrap()) }
    fn pk(&mut self) -> Result<Pubkey, ProgramError> { Ok(Pubkey::new_from_array(self.arr32()?)) }
    fn bytes_str(&mut self) -> Result<&'a [u8], ProgramError> { let n = self.u32()? as usize; self.take(n) }
}

struct Out(Vec<u8>);
impl Out {
    fn new(d: &[u8; 8]) -> Self { let mut v = Vec::with_capacity(256); v.extend_from_slice(d); Out(v) }
    fn u8(&mut self, v: u8) -> &mut Self { self.0.push(v); self }
    fn u16(&mut self, v: u16) -> &mut Self { self.0.extend_from_slice(&v.to_le_bytes()); self }
    fn u32(&mut self, v: u32) -> &mut Self { self.0.extend_from_slice(&v.to_le_bytes()); self }
    fn u64(&mut self, v: u64) -> &mut Self { self.0.extend_from_slice(&v.to_le_bytes()); self }
    fn i64(&mut self, v: i64) -> &mut Self { self.0.extend_from_slice(&v.to_le_bytes()); self }
    fn u128(&mut self, v: u128) -> &mut Self { self.0.extend_from_slice(&v.to_le_bytes()); self }
    fn raw(&mut self, v: &[u8]) -> &mut Self { self.0.extend_from_slice(v); self }
    fn pk(&mut self, v: &Pubkey) -> &mut Self { self.raw(v.as_ref()) }
    fn bytes_str(&mut self, v: &[u8]) -> &mut Self { self.u32(v.len() as u32).raw(v) }
    fn emit(&self) { sol_log_data(&[&self.0]); }
    fn save(&self, ai: &AccountInfo) -> ProgramResult {
        let mut d = ai.try_borrow_mut_data()?;
        if self.0.len() > d.len() { return Err(ProgramError::AccountDataTooSmall); }
        d[..self.0.len()].copy_from_slice(&self.0);
        Ok(())
    }
}

/* ---------------- account state ---------------- */

struct House {
    authority: Pubkey, treasury: Pubkey,
    bankroll: u64, escrow: u64, liability: u64, owed: u64,
    total_shares: u128, acc_per_share: u128, hwm_nav: u128,
    last_payout: i64, payout_interval: i64, max_profit_bps: u16, platform_fee_bps: u16,
    wagered: u64, bets: u64, fed: u64, paid_to_coins: u64, paid_to_platform: u64, bump: u8, paused: bool,
}
#[derive(Clone, Copy, Default)]
struct RaceEntry { coin: Pubkey, score: u64 }
struct Table { id: u8, bump: u8, sponsor: Pubkey, updated: i64, wagered: u64, bets: u64, race: [RaceEntry; 8] }
struct Coin {
    mint: Pubkey, creator: Pubkey, buyback_wallet: Pubkey, burn_bps: u16, home_table: u8,
    shares: u128, debt: u128, claimable: u64, fed_total: u64, claimed_total: u64,
    name: Vec<u8>, symbol: Vec<u8>, uri: Vec<u8>, bump: u8,
}
struct Bet {
    player: Pubkey, id: u64, table_id: u8, game: u8, a: u8, b: u8, target_e2: u32,
    amount: u64, max_payout: u64, slot: u64, client_seed: [u8; 32], bump: u8,
}

fn open<'a>(ai: &'a AccountInfo, pid: &Pubkey, disc: &[u8; 8]) -> Result<std::cell::Ref<'a, &'a mut [u8]>, ProgramError> {
    require!(ai.owner == pid, HpError::BadAccount);
    let d = ai.try_borrow_data()?;
    require!(d.len() >= 8 && d[..8] == disc[..], HpError::BadAccount);
    Ok(d)
}
fn check_pda(key: &Pubkey, seeds: &[&[u8]], pid: &Pubkey) -> ProgramResult {
    let k = Pubkey::create_program_address(seeds, pid).map_err(|_| fail(HpError::BadAccount))?;
    require!(&k == key, HpError::BadAccount);
    Ok(())
}

impl House {
    fn load(ai: &AccountInfo, pid: &Pubkey) -> Result<House, ProgramError> {
        let h = {
            let d = open(ai, pid, &ACC_HOUSE)?;
            let mut r = Rd { b: &d, o: 8 };
            House {
                authority: r.pk()?, treasury: r.pk()?,
                bankroll: r.u64()?, escrow: r.u64()?, liability: r.u64()?, owed: r.u64()?,
                total_shares: r.u128()?, acc_per_share: r.u128()?, hwm_nav: r.u128()?,
                last_payout: r.i64()?, payout_interval: r.i64()?, max_profit_bps: r.u16()?, platform_fee_bps: r.u16()?,
                wagered: r.u64()?, bets: r.u64()?, fed: r.u64()?, paid_to_coins: r.u64()?, paid_to_platform: r.u64()?, bump: r.u8()?,
                paused: r.u8()? != 0,
            }
        };
        check_pda(ai.key, &[b"house", &[h.bump]], pid)?;
        Ok(h)
    }
    fn save(&self, ai: &AccountInfo) -> ProgramResult {
        let mut o = Out::new(&ACC_HOUSE);
        o.pk(&self.authority).pk(&self.treasury)
            .u64(self.bankroll).u64(self.escrow).u64(self.liability).u64(self.owed)
            .u128(self.total_shares).u128(self.acc_per_share).u128(self.hwm_nav)
            .i64(self.last_payout).i64(self.payout_interval).u16(self.max_profit_bps).u16(self.platform_fee_bps)
            .u64(self.wagered).u64(self.bets).u64(self.fed).u64(self.paid_to_coins).u64(self.paid_to_platform).u8(self.bump)
            .u8(self.paused as u8);
        o.save(ai)
    }
}

impl Table {
    fn load(ai: &AccountInfo, pid: &Pubkey, id: u8) -> Result<Table, ProgramError> {
        let t = {
            let d = open(ai, pid, &ACC_TABLE)?;
            let mut r = Rd { b: &d, o: 8 };
            let mut t = Table { id: r.u8()?, bump: r.u8()?, sponsor: r.pk()?, updated: r.i64()?, wagered: r.u64()?, bets: r.u64()?, race: [RaceEntry::default(); 8] };
            for e in t.race.iter_mut() { e.coin = r.pk()?; e.score = r.u64()?; }
            t
        };
        check_pda(ai.key, &[b"table", &[id], &[t.bump]], pid)?;
        Ok(t)
    }
    fn save(&self, ai: &AccountInfo) -> ProgramResult {
        let mut o = Out::new(&ACC_TABLE);
        o.u8(self.id).u8(self.bump).pk(&self.sponsor).i64(self.updated).u64(self.wagered).u64(self.bets);
        for e in self.race.iter() { o.pk(&e.coin).u64(e.score); }
        o.save(ai)
    }
}

impl Coin {
    fn load(ai: &AccountInfo, pid: &Pubkey) -> Result<Coin, ProgramError> {
        let c = {
            let d = open(ai, pid, &ACC_COIN)?;
            let mut r = Rd { b: &d, o: 8 };
            Coin {
                mint: r.pk()?, creator: r.pk()?, buyback_wallet: r.pk()?, burn_bps: r.u16()?, home_table: r.u8()?,
                shares: r.u128()?, debt: r.u128()?, claimable: r.u64()?, fed_total: r.u64()?, claimed_total: r.u64()?,
                name: r.bytes_str()?.to_vec(), symbol: r.bytes_str()?.to_vec(), uri: r.bytes_str()?.to_vec(), bump: r.u8()?,
            }
        };
        check_pda(ai.key, &[b"coin", c.mint.as_ref(), &[c.bump]], pid)?;
        Ok(c)
    }
    fn save(&self, ai: &AccountInfo) -> ProgramResult {
        let mut o = Out::new(&ACC_COIN);
        o.pk(&self.mint).pk(&self.creator).pk(&self.buyback_wallet).u16(self.burn_bps).u8(self.home_table)
            .u128(self.shares).u128(self.debt).u64(self.claimable).u64(self.fed_total).u64(self.claimed_total)
            .bytes_str(&self.name).bytes_str(&self.symbol).bytes_str(&self.uri).u8(self.bump);
        o.save(ai)
    }
}

impl Bet {
    fn load(ai: &AccountInfo, pid: &Pubkey) -> Result<Bet, ProgramError> {
        let b = {
            let d = open(ai, pid, &ACC_BET)?;
            let mut r = Rd { b: &d, o: 8 };
            Bet {
                player: r.pk()?, id: r.u64()?, table_id: r.u8()?, game: r.u8()?, a: r.u8()?, b: r.u8()?, target_e2: r.u32()?,
                amount: r.u64()?, max_payout: r.u64()?, slot: r.u64()?, client_seed: r.arr32()?, bump: r.u8()?,
            }
        };
        check_pda(ai.key, &[b"bet", b.player.as_ref(), &b.id.to_le_bytes(), &[b.bump]], pid)?;
        Ok(b)
    }
    fn save(&self, ai: &AccountInfo) -> ProgramResult {
        let mut o = Out::new(&ACC_BET);
        o.pk(&self.player).u64(self.id).u8(self.table_id).u8(self.game).u8(self.a).u8(self.b).u32(self.target_e2)
            .u64(self.amount).u64(self.max_payout).u64(self.slot).raw(&self.client_seed).u8(self.bump);
        o.save(ai)
    }
}

/* ---------------- account helpers ---------------- */

fn signer(ai: &AccountInfo) -> ProgramResult {
    if !ai.is_signer { return Err(ProgramError::MissingRequiredSignature); }
    Ok(())
}
fn system(ai: &AccountInfo) -> ProgramResult {
    require!(ai.key == &system_program::ID, HpError::BadAccount);
    Ok(())
}

/// Creates a program-owned PDA account (handles an address someone pre-funded, like Anchor's init).
fn create_pda<'a>(payer: &AccountInfo<'a>, target: &AccountInfo<'a>, sys: &AccountInfo<'a>, space: usize, pid: &Pubkey, seeds: &[&[u8]]) -> ProgramResult {
    let need = Rent::get()?.minimum_balance(space);
    let have = target.lamports();
    if have == 0 {
        invoke_signed(&system_instruction::create_account(payer.key, target.key, need, space as u64, pid),
            &[payer.clone(), target.clone(), sys.clone()], &[seeds])?;
    } else {
        require!(target.owner == &system_program::ID && target.data_is_empty(), HpError::BadAccount);
        if need > have {
            invoke(&system_instruction::transfer(payer.key, target.key, need - have), &[payer.clone(), target.clone(), sys.clone()])?;
        }
        invoke_signed(&system_instruction::allocate(target.key, space as u64), &[target.clone(), sys.clone()], &[seeds])?;
        invoke_signed(&system_instruction::assign(target.key, pid), &[target.clone(), sys.clone()], &[seeds])?;
    }
    Ok(())
}

fn move_lamports(from: &AccountInfo, to: &AccountInfo, amount: u64) -> ProgramResult {
    let from_new = from.lamports().checked_sub(amount).ok_or_else(math)?;
    let to_new = to.lamports().checked_add(amount).ok_or_else(math)?;
    **from.try_borrow_mut_lamports()? = from_new;
    **to.try_borrow_mut_lamports()? = to_new;
    Ok(())
}

/* ---------------- entrypoint ---------------- */

pub fn process_instruction(pid: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    require!(data.len() >= 8, HpError::BadParam);
    let disc: [u8; 8] = data[..8].try_into().unwrap();
    let mut r = Rd { b: data, o: 8 };
    match disc {
        IX_INITIALIZE => initialize(pid, accounts, r.i64()?, r.u16()?, r.u16()?),
        IX_INIT_TABLE => init_table(pid, accounts, r.u8()?),
        IX_SET_PARAMS => set_params(pid, accounts, r.i64()?, r.u16()?, r.u16()?),
        IX_REGISTER_COIN => {
            let name = r.bytes_str()?;
            let symbol = r.bytes_str()?;
            let uri = r.bytes_str()?;
            register_coin(pid, accounts, name, symbol, uri, r.u16()?, r.u8()?)
        }
        IX_FEED => feed(pid, accounts, r.u8()?, r.u64()?),
        IX_SWEEP_FEES => sweep_fees(pid, accounts),
        IX_PLACE_BET => {
            let bet_id = r.u64()?;
            let table_id = r.u8()?;
            let game = r.u8()?;
            let a = r.u8()?;
            let b = r.u8()?;
            let target_e2 = r.u32()?;
            let amount = r.u64()?;
            let seed = r.arr32()?;
            place_bet(pid, accounts, bet_id, table_id, game, a, b, target_e2, amount, seed)
        }
        IX_SETTLE_BET => settle_bet(pid, accounts),
        IX_CRANK_PAYOUT => crank_payout(pid, accounts),
        IX_CLAIM_COIN => claim_coin(pid, accounts),
        IX_SET_PAUSED => set_paused(pid, accounts, r.u8()? != 0),
        IX_EMERGENCY_WITHDRAW => emergency_withdraw(pid, accounts, r.u64()?),
        IX_SET_AUTHORITY => set_authority(pid, accounts, r.pk()?),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

/* ---------------- instructions ---------------- */

fn check_params(payout_interval: i64, max_profit_bps: u16, platform_fee_bps: u16) -> ProgramResult {
    require!(payout_interval >= 30, HpError::BadParam);
    require!(max_profit_bps > 0 && max_profit_bps <= 1000, HpError::BadParam);
    require!(platform_fee_bps <= 5000, HpError::BadParam);
    Ok(())
}

fn initialize(pid: &Pubkey, accounts: &[AccountInfo], _payout_interval: i64, _max_profit_bps: u16, _platform_fee_bps: u16) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let payer = next_account_info(it)?;
    let _treasury = next_account_info(it)?; // kept for layout compatibility; the treasury starts as the owner
    let sys = next_account_info(it)?;
    signer(payer)?;
    system(sys)?;
    // whoever pays to create the house gets no say in its settings: fixed defaults, owner-adjustable later
    let (payout_interval, max_profit_bps, platform_fee_bps) = (DEFAULT_PAYOUT_INTERVAL, DEFAULT_MAX_PROFIT_BPS, DEFAULT_PLATFORM_FEE_BPS);
    let (key, bump) = Pubkey::find_program_address(&[b"house"], pid);
    require!(&key == house.key, HpError::BadAccount);
    create_pda(payer, house, sys, HOUSE_SPACE, pid, &[b"house", &[bump]])?;
    // anyone may pay to create the house, but the owner wallet is always its admin and treasury
    House {
        authority: admin(), treasury: admin(),
        bankroll: 0, escrow: 0, liability: 0, owed: 0,
        total_shares: 0, acc_per_share: 0, hwm_nav: SCALE,
        last_payout: Clock::get()?.unix_timestamp, payout_interval, max_profit_bps, platform_fee_bps,
        wagered: 0, bets: 0, fed: 0, paid_to_coins: 0, paid_to_platform: 0, bump, paused: false,
    }.save(house)
}

fn init_table(pid: &Pubkey, accounts: &[AccountInfo], id: u8) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let table = next_account_info(it)?;
    let payer = next_account_info(it)?;
    let sys = next_account_info(it)?;
    signer(payer)?;
    system(sys)?;
    House::load(house, pid)?; // tables are fixed PDAs, so anyone may pay to create them
    require!(id < NUM_TABLES, HpError::BadTable);
    let (key, bump) = Pubkey::find_program_address(&[b"table", &[id]], pid);
    require!(&key == table.key, HpError::BadAccount);
    create_pda(payer, table, sys, TABLE_SPACE, pid, &[b"table", &[id], &[bump]])?;
    Table { id, bump, sponsor: Pubkey::default(), updated: Clock::get()?.unix_timestamp, wagered: 0, bets: 0, race: [RaceEntry::default(); 8] }.save(table)
}

fn set_params(pid: &Pubkey, accounts: &[AccountInfo], payout_interval: i64, max_profit_bps: u16, platform_fee_bps: u16) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let authority = next_account_info(it)?;
    let treasury = next_account_info(it)?;
    signer(authority)?;
    check_params(payout_interval, max_profit_bps, platform_fee_bps)?;
    let mut h = House::load(house, pid)?;
    require!(&h.authority == authority.key, HpError::NotOwner);
    h.payout_interval = payout_interval;
    h.max_profit_bps = max_profit_bps;
    h.platform_fee_bps = platform_fee_bps;
    h.treasury = *treasury.key;
    h.save(house)
}

fn register_coin(pid: &Pubkey, accounts: &[AccountInfo], name: &[u8], symbol: &[u8], uri: &[u8], burn_bps: u16, home_table: u8) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let coin = next_account_info(it)?;
    let mint = next_account_info(it)?;
    let buyback = next_account_info(it)?;
    let creator = next_account_info(it)?;
    let sys = next_account_info(it)?;
    let intake = next_account_info(it)?;
    let curve = next_account_info(it)?;
    signer(creator)?;
    // the new coin's mint key must sign, so a coin can only be registered as part of its own launch
    signer(mint)?;
    system(sys)?;
    House::load(house, pid)?;
    // and the coin must really exist on pump.fun: its bonding curve is created earlier in the same transaction
    let (curve_key, _) = Pubkey::find_program_address(&[b"bonding-curve", mint.key.as_ref()], &PUMP_PROGRAM);
    require!(&curve_key == curve.key && curve.owner == &PUMP_PROGRAM && !curve.data_is_empty(), HpError::NotPumpCoin);
    require!(name.len() <= 32 && symbol.len() <= 13 && uri.len() <= 200, HpError::TooLong);
    require!(burn_bps <= 10_000, HpError::BadParam);
    require!(home_table < NUM_TABLES, HpError::BadTable);
    let (key, bump) = Pubkey::find_program_address(&[b"coin", mint.key.as_ref()], pid);
    require!(&key == coin.key, HpError::BadAccount);
    create_pda(creator, coin, sys, COIN_SPACE, pid, &[b"coin", mint.key.as_ref(), &[bump]])?;
    Coin {
        mint: *mint.key, creator: *creator.key, buyback_wallet: *buyback.key, burn_bps, home_table,
        shares: 0, debt: 0, claimable: 0, fed_total: 0, claimed_total: 0,
        name: name.to_vec(), symbol: symbol.to_vec(), uri: uri.to_vec(), bump,
    }.save(coin)?;
    // open the coin's fee-intake address with its rent deposit, so pump.fun can pay creator fees into it
    let (fees_key, _) = Pubkey::find_program_address(&[b"fees", mint.key.as_ref()], pid);
    require!(&fees_key == intake.key && intake.owner == &system_program::ID, HpError::BadAccount);
    let floor = Rent::get()?.minimum_balance(0);
    if intake.lamports() < floor {
        invoke(&system_instruction::transfer(creator.key, intake.key, floor - intake.lamports()),
            &[creator.clone(), intake.clone(), sys.clone()])?;
    }
    Out::new(&EV_COIN_REGISTERED).pk(mint.key).pk(creator.key).emit();
    Ok(())
}

/// Anyone adds SOL to the house in a coin's name, aimed at one table's race.
fn feed(pid: &Pubkey, accounts: &[AccountInfo], table_id: u8, amount: u64) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let coin = next_account_info(it)?;
    let table = next_account_info(it)?;
    let feeder = next_account_info(it)?;
    let sys = next_account_info(it)?;
    signer(feeder)?;
    system(sys)?;
    require!(amount > 0, HpError::ZeroAmount);
    let mut h = House::load(house, pid)?;
    require!(!h.paused, HpError::Paused);
    let mut c = Coin::load(coin, pid)?;
    let mut t = Table::load(table, pid, table_id)?;
    invoke(&system_instruction::transfer(feeder.key, house.key, amount), &[feeder.clone(), house.clone(), sys.clone()])?;
    let now = Clock::get()?.unix_timestamp;
    let shares = credit_feed(&mut h, &mut c, &mut t, amount, now)?;
    h.save(house)?;
    c.save(coin)?;
    t.save(table)?;
    Out::new(&EV_FED).pk(&c.mint).u8(table_id).u64(amount).u64(shares as u64).pk(feeder.key).pk(&t.sponsor).emit();
    Ok(())
}

/// Moves whatever has landed at a coin's fee-intake address (its routed creator fees) into the house.
fn sweep_fees(pid: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let coin = next_account_info(it)?;
    let table = next_account_info(it)?;
    let intake = next_account_info(it)?;
    let cranker = next_account_info(it)?;
    let sys = next_account_info(it)?;
    signer(cranker)?;
    system(sys)?;
    let mut h = House::load(house, pid)?;
    require!(!h.paused, HpError::Paused);
    let mut c = Coin::load(coin, pid)?;
    let mut t = Table::load(table, pid, c.home_table)?;
    let (key, bump) = Pubkey::find_program_address(&[b"fees", c.mint.as_ref()], pid);
    require!(&key == intake.key && intake.owner == &system_program::ID, HpError::BadAccount);
    // always leave the rent deposit, so the intake stays open for pump.fun's next payout
    let amount = intake.lamports().saturating_sub(Rent::get()?.minimum_balance(0));
    require!(amount > 0, HpError::ZeroAmount);
    invoke_signed(&system_instruction::transfer(intake.key, house.key, amount),
        &[intake.clone(), house.clone(), sys.clone()], &[&[b"fees", c.mint.as_ref(), &[bump]]])?;
    let now = Clock::get()?.unix_timestamp;
    let shares = credit_feed(&mut h, &mut c, &mut t, amount, now)?;
    h.save(house)?;
    c.save(coin)?;
    t.save(table)?;
    Out::new(&EV_FED).pk(&c.mint).u8(t.id).u64(amount).u64(shares as u64).pk(intake.key).pk(&t.sponsor).emit();
    Ok(())
}

fn place_bet(pid: &Pubkey, accounts: &[AccountInfo], bet_id: u64, table_id: u8, game: u8, a: u8, b: u8, target_e2: u32, amount: u64, client_seed: [u8; 32]) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let table = next_account_info(it)?;
    let bet_ai = next_account_info(it)?;
    let player = next_account_info(it)?;
    let sys = next_account_info(it)?;
    signer(player)?;
    system(sys)?;
    let mut h = House::load(house, pid)?;
    require!(!h.paused, HpError::Paused);
    let mut t = Table::load(table, pid, table_id)?;

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
        _ => return Err(fail(HpError::GameNotOnTable)),
    };
    let max_payout = (amount as u128 * top_e2 as u128 / 100) as u64;
    let free = h.bankroll.saturating_sub(h.liability);
    let profit_cap = (free as u128 * h.max_profit_bps as u128 / 10_000) as u64;
    require!(max_payout.saturating_sub(amount) <= profit_cap, HpError::OverMaxWin);

    let id_le = bet_id.to_le_bytes();
    let (key, bump) = Pubkey::find_program_address(&[b"bet", player.key.as_ref(), &id_le], pid);
    require!(&key == bet_ai.key, HpError::BadAccount);
    create_pda(player, bet_ai, sys, BET_SPACE, pid, &[b"bet", player.key.as_ref(), &id_le, &[bump]])?;
    invoke(&system_instruction::transfer(player.key, house.key, amount), &[player.clone(), house.clone(), sys.clone()])?;

    h.escrow = h.escrow.checked_add(amount).ok_or_else(math)?;
    h.liability = h.liability.checked_add(max_payout.saturating_sub(amount)).ok_or_else(math)?;
    h.wagered = h.wagered.saturating_add(amount);
    h.bets = h.bets.saturating_add(1);
    t.wagered = t.wagered.saturating_add(amount);
    t.bets = t.bets.saturating_add(1);
    let slot = Clock::get()?.slot;
    h.save(house)?;
    t.save(table)?;
    Bet { player: *player.key, id: bet_id, table_id, game, a, b, target_e2, amount, max_payout, slot, client_seed, bump }.save(bet_ai)?;
    Out::new(&EV_BET_PLACED).pk(player.key).pk(bet_ai.key).u8(table_id).u8(game).u64(amount).u64(slot).emit();
    Ok(())
}

/// Anyone can settle. The result uses the hash of the first slot after the bet's slot.
/// If that hash has rolled out of the SlotHashes window, the bet expired and counts as a loss.
fn settle_bet(pid: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let bet_ai = next_account_info(it)?;
    let player = next_account_info(it)?;
    let hashes = next_account_info(it)?;
    let cranker = next_account_info(it)?;
    signer(cranker)?;
    require!(hashes.key == &slot_hashes::ID, HpError::BadSysvar);
    let mut h = House::load(house, pid)?;
    let bet = Bet::load(bet_ai, pid)?;
    require!(&bet.player == player.key, HpError::BadAccount);

    let current = Clock::get()?.slot;
    require!(current > bet.slot + 1, HpError::TooEarly);
    // SlotHashes lists recent slots newest first. The result slot is the first slot after the bet's
    // that produced a block. If that slot has already left the window, the bet expired (a loss).
    let mut found: Option<([u8; 32], u64)> = None;
    let mut expired = true;
    {
        let data = hashes.try_borrow_data()?;
        require!(data.len() >= 8, HpError::BadSysvar);
        let n = u64::from_le_bytes(data[0..8].try_into().unwrap()) as usize;
        for i in 0..n {
            let off = 8 + i * 40;
            if off + 40 > data.len() { break; }
            let s = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
            if s > bet.slot {
                found = Some((data[off + 8..off + 40].try_into().unwrap(), s));
            } else {
                expired = false;
                break;
            }
        }
    }
    // the window's oldest entry being the very next slot also proves nothing was skipped
    if let Some((_, s)) = found { if s == bet.slot + 1 { expired = false; } }
    let found = found.map(|(hsh, _)| hsh);
    let (payout, roll) = match (found, expired) {
        (Some(sh), false) => {
            let r = hashv(&[&sh, &bet.client_seed, bet_ai.key.as_ref()]).to_bytes();
            (outcome(&bet, &r), r)
        }
        (None, false) => return Err(fail(HpError::TooEarly)),
        _ => (0u64, [0u8; 32]),
    };

    let amount = bet.amount;
    // escrowed stake joins the bankroll, then the payout leaves it
    // (after an emergency withdraw the stake may already be gone, so only what escrow still holds moves)
    let back = amount.min(h.escrow);
    h.escrow -= back;
    h.liability = h.liability.saturating_sub(bet.max_payout.saturating_sub(amount));
    h.bankroll = h.bankroll.checked_add(back).ok_or_else(math)?;
    // in normal operation bankroll always covers the payout (see place_bet); the cap only matters after an owner withdrawal
    let payout = payout.min(h.bankroll);
    h.bankroll -= payout;
    h.save(house)?;
    if payout > 0 {
        move_lamports(house, player, payout)?;
    }
    // close the bet: wipe its data and return its rent to the player
    {
        let mut d = bet_ai.try_borrow_mut_data()?;
        for x in d.iter_mut() { *x = 0; }
    }
    move_lamports(bet_ai, player, bet_ai.lamports())?;
    Out::new(&EV_BET_SETTLED).pk(&bet.player).pk(bet_ai.key).u8(bet.table_id).u8(bet.game).u64(amount).u64(payout).u8(expired as u8).raw(&roll).emit();
    Ok(())
}

/// Permissionless. Pays profit above the high-water mark: platform fee out, the rest to coins by share.
fn crank_payout(pid: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let treasury = next_account_info(it)?;
    let cranker = next_account_info(it)?;
    signer(cranker)?;
    let mut h = House::load(house, pid)?;
    require!(&h.treasury == treasury.key, HpError::BadAccount);
    let now = Clock::get()?.unix_timestamp;
    require!(now >= h.last_payout + h.payout_interval, HpError::TooEarly);
    h.last_payout = now;
    if h.total_shares == 0 { return h.save(house); }
    let nav = h.bankroll as u128 * SCALE / h.total_shares;
    if nav <= h.hwm_nav {
        h.save(house)?;
        Out::new(&EV_PAYOUT_SKIPPED).u64(((h.hwm_nav - nav) * h.total_shares / SCALE) as u64).emit();
        return Ok(());
    }
    let mut profit = ((nav - h.hwm_nav) * h.total_shares / SCALE) as u64;
    // never pay out SOL that open bets could still win; the rest waits for the next payout
    profit = profit.min(h.bankroll.saturating_sub(h.liability));
    if profit == 0 { return h.save(house); }
    let fee = (profit as u128 * h.platform_fee_bps as u128 / 10_000) as u64;
    let to_coins = profit - fee;
    h.bankroll = h.bankroll.checked_sub(profit).ok_or_else(math)?;
    h.owed = h.owed.checked_add(to_coins).ok_or_else(math)?;
    h.acc_per_share = h.acc_per_share.checked_add(to_coins as u128 * SCALE / h.total_shares).ok_or_else(math)?;
    h.paid_to_coins = h.paid_to_coins.saturating_add(to_coins);
    h.paid_to_platform = h.paid_to_platform.saturating_add(fee);
    h.save(house)?;
    if fee > 0 {
        move_lamports(house, treasury, fee)?;
    }
    Out::new(&EV_PAYOUT).u64(profit).u64(fee).u64(to_coins).u64(nav as u64).emit();
    Ok(())
}

/// Permissionless. Sends a coin's earned profit to its buyback wallet, which buys the coin
/// on the market and burns it or pays stakers on the coin's split.
fn claim_coin(pid: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let coin = next_account_info(it)?;
    let buyback = next_account_info(it)?;
    let cranker = next_account_info(it)?;
    signer(cranker)?;
    let mut h = House::load(house, pid)?;
    let mut c = Coin::load(coin, pid)?;
    require!(&c.buyback_wallet == buyback.key, HpError::BadAccount);
    accrue(&h, &mut c);
    // capped at what the house holds for coins, so per-share rounding can never pull from the bankroll
    let amount = c.claimable.min(h.owed);
    require!(amount > 0, HpError::ZeroAmount);
    c.claimable -= amount;
    c.claimed_total = c.claimed_total.saturating_add(amount);
    h.owed -= amount;
    h.save(house)?;
    c.save(coin)?;
    move_lamports(house, buyback, amount)?;
    Out::new(&EV_CLAIMED).pk(&c.mint).u64(amount).pk(&c.buyback_wallet).emit();
    Ok(())
}

/* ---------------- owner fail-safe ---------------- */

/// Owner only. Paused: no new bets, feeds or fee sweeps. Settling, payouts and claims keep working.
fn set_paused(pid: &Pubkey, accounts: &[AccountInfo], paused: bool) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let authority = next_account_info(it)?;
    signer(authority)?;
    let mut h = House::load(house, pid)?;
    require!(&h.authority == authority.key, HpError::NotOwner);
    h.paused = paused;
    h.save(house)?;
    Out::new(&EV_PAUSED).u8(paused as u8).emit();
    Ok(())
}

/// Owner only, and only while paused. Sends SOL from the house to any wallet.
/// `amount` 0 means everything the house holds for itself and coins. Players' stakes on open bets are
/// never touched, so every open bet can still be settled and refunded.
/// It comes out of the bankroll first, then coin profits not yet claimed.
fn emergency_withdraw(pid: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let authority = next_account_info(it)?;
    let to = next_account_info(it)?;
    signer(authority)?;
    let mut h = House::load(house, pid)?;
    require!(&h.authority == authority.key, HpError::NotOwner);
    require!(h.paused, HpError::NotPaused);
    let spare = house.lamports().saturating_sub(Rent::get()?.minimum_balance(house.data_len())).saturating_sub(h.escrow);
    let available = spare.min(h.bankroll.saturating_add(h.owed));
    let amount = if amount == 0 { available } else { amount.min(available) };
    require!(amount > 0, HpError::ZeroAmount);
    let mut left = amount;
    let from_bank = left.min(h.bankroll); h.bankroll -= from_bank; left -= from_bank;
    let from_owed = left.min(h.owed); h.owed -= from_owed;
    h.liability = h.liability.min(h.bankroll);
    h.save(house)?;
    move_lamports(house, to, amount)?;
    Out::new(&EV_EMERGENCY_WITHDRAW).u64(amount).pk(to.key).emit();
    Ok(())
}

/// Owner only. Hands the house admin role (and nothing else) to another wallet.
fn set_authority(pid: &Pubkey, accounts: &[AccountInfo], new_authority: Pubkey) -> ProgramResult {
    let it = &mut accounts.iter();
    let house = next_account_info(it)?;
    let authority = next_account_info(it)?;
    signer(authority)?;
    let mut h = House::load(house, pid)?;
    require!(&h.authority == authority.key, HpError::NotOwner);
    require!(new_authority != Pubkey::default(), HpError::BadParam);
    h.authority = new_authority;
    h.save(house)?;
    Out::new(&EV_AUTHORITY_CHANGED).pk(authority.key).pk(&new_authority).emit();
    Ok(())
}

/* ---------------- helpers (same as the Anchor build) ---------------- */

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
            let sym = |i: usize| slot_symbol(u32::from_le_bytes(r[i * 4..i * 4 + 4].try_into().unwrap()));
            let s = [sym(0), sym(1), sym(2)];
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
fn crash_e2(r: &[u8; 32]) -> u64 {
    let x = u64::from_le_bytes(r[0..8].try_into().unwrap()) as u128;
    let two64: u128 = 1u128 << 64;
    let v = 99u128 * two64 / (two64 - x);
    (v.min(MAX_CRASH_E2 as u128) as u64).max(100)
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
fn credit_feed(h: &mut House, c: &mut Coin, t: &mut Table, amount: u64, now: i64) -> Result<u128, ProgramError> {
    accrue(h, c);
    // shares are priced at the current value per share, never below 0.1% of the payout line (so a
    // near-wiped-out house can't be diluted to nothing); with an empty bankroll they're priced at the line
    let nav = if h.total_shares == 0 || h.bankroll == 0 { h.hwm_nav } else { (h.bankroll as u128 * SCALE / h.total_shares).max(h.hwm_nav / 1000) };
    let shares = amount as u128 * SCALE / nav;
    h.bankroll = h.bankroll.checked_add(amount).ok_or_else(math)?;
    h.total_shares = h.total_shares.checked_add(shares).ok_or_else(math)?;
    h.fed = h.fed.saturating_add(amount);
    c.shares = c.shares.checked_add(shares).ok_or_else(math)?;
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
