// GET /api/config -> what the launch form needs: Housepad's share, the casino-stake minimum, limits.
const { env, handler, MAX_SHAREHOLDERS } = require('./_lib.js');
const { houseIsOpen } = require('./_housepad.js');
module.exports = handler(async () => {
  const { connection, platform, platformBps, houseMinBps, programId } = env();
  const house = await houseIsOpen(connection, programId).catch(() => ({ open: false, paused: false }));
  return {
    platformWallet: platform.toBase58(), platformBps, houseMinBps, programId: programId.toBase58(),
    maxCreatorWallets: MAX_SHAREHOLDERS - (platformBps > 0 ? 1 : 0) - 1, houseOpen: house.open, housePaused: house.paused,
  };
});
