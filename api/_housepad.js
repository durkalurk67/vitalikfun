// Housepad program helpers for the launch API: addresses and the register_coin instruction.
const { PublicKey, SystemProgram, TransactionInstruction } = require('@solana/web3.js');

const IX_REGISTER_COIN = Buffer.from([79, 37, 188, 46, 209, 104, 90, 10]);
const ACC_HOUSE = Buffer.from([21, 145, 94, 109, 254, 199, 210, 151]);
const NUM_TABLES = 8;
const PUMP_PROGRAM = new PublicKey('6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P');
// A system account with no data needs this much to exist; pump.fun only pays fees into existing accounts.
const INTAKE_RENT_LAMPORTS = 890_880;

const pda = (programId, seeds) => PublicKey.findProgramAddressSync(seeds, programId)[0];
const housePda = (programId) => pda(programId, [Buffer.from('house')]);
const coinPda = (programId, mint) => pda(programId, [Buffer.from('coin'), mint.toBuffer()]);
// The coin's casino-stake address: its share of creator fees lands here and is swept into the bankroll.
const feesPda = (programId, mint) => pda(programId, [Buffer.from('fees'), mint.toBuffer()]);
// pump.fun's bonding curve for the coin; the program checks it exists, so only real pump.fun coins can join.
const bondingCurvePda = (mint) => pda(PUMP_PROGRAM, [Buffer.from('bonding-curve'), mint.toBuffer()]);

function borshString(s) {
  const b = Buffer.from(s, 'utf8');
  const len = Buffer.alloc(4); len.writeUInt32LE(b.length);
  return Buffer.concat([len, b]);
}

// register_coin(name, symbol, uri, burn_bps, home_table). The new mint must sign.
function registerCoinIx({ programId, creator, mint, buyback, name, symbol, uri, burnBps, homeTable }) {
  const burn = Buffer.alloc(2); burn.writeUInt16LE(burnBps);
  const data = Buffer.concat([IX_REGISTER_COIN, borshString(name), borshString(symbol), borshString(uri), burn, Buffer.from([homeTable])]);
  return new TransactionInstruction({
    programId,
    keys: [
      { pubkey: housePda(programId), isSigner: false, isWritable: false },
      { pubkey: coinPda(programId, mint), isSigner: false, isWritable: true },
      { pubkey: mint, isSigner: true, isWritable: false },
      { pubkey: buyback, isSigner: false, isWritable: false },
      { pubkey: creator, isSigner: true, isWritable: true },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      { pubkey: feesPda(programId, mint), isSigner: false, isWritable: true },
      { pubkey: bondingCurvePda(mint), isSigner: false, isWritable: false },
    ],
    data,
  });
}

async function houseIsOpen(connection, programId) {
  const a = await connection.getAccountInfo(housePda(programId));
  if (!a || !a.owner.equals(programId) || a.data.length < 214 || !a.data.subarray(0, 8).equals(ACC_HOUSE)) return { open: false, paused: false };
  return { open: true, paused: a.data[213] === 1 };
}

module.exports = { NUM_TABLES, INTAKE_RENT_LAMPORTS, PUMP_PROGRAM, housePda, coinPda, feesPda, bondingCurvePda, registerCoinIx, houseIsOpen };
