# NEESS ColorOS 17 FOD & AOD Fix v5.0 — Xiaomi 12X (`psyche`)

Adapted from [NEESCHAL-3/NEESS-COS-AOD-FOD-FIX](https://github.com/NEESCHAL-3/NEESS-COS-AOD-FOD-FIX) for Xiaomi 12X (`psyche`, SM8250-AC, `4.19` kernel) running ColorOS 17 (`SDK 37`).

## What Changed in v5.0
- **Session-Gated FOD HBM**: Tracks active `FingerprintEnrollClient` / `FingerprintAuthenticationClient` / `FingerprintDetectClient` sessions (via `BiometricScheduler`, `BiometricStateCallback`, `OnScreenFingerprintUiMech`, plus an initial `dumpsys fingerprint` check at boot). Touching the sensor area while awake with no active fingerprint prompt no longer triggers HBM or `extCmd`.
- Automatically preserves your clean `vendor.xiaomi.hardware.fx.tunnel@1.0.so` when flashed over v4.
