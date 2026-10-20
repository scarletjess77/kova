// Where donations go. One place, so the address can't drift between screens.
export const TON_ADDRESS = 'UQCfiJW3GM9sEfYS2fsk-q9WNbKXXn3m0KU17Dx5XTCs3fv0';

/** Standard TON payment link: wallets open it when the QR code is scanned. */
export const TON_URI = `ton://transfer/${TON_ADDRESS}`;

/** Opens Tonkeeper (the app if installed, otherwise its web wallet). */
export const TONKEEPER_URL = `https://app.tonkeeper.com/transfer/${TON_ADDRESS}`;
