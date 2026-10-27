# Gate-2 artifact verification records (b587d46f, 2026-10-25)

Provenance: the two-party arrival gate for the one-shot download artifact
`lpac-mt7621-gate2-mipsel_24kc.tar.gz`
(sha256 `b587d46fbdcbb6f21d48337651247eaf69d0e6b9a1ccbb3ac3c8250007aa9b44`,
branch `route1-gate2` = pin `a43e46ae` → uppercase `1982d5e` → #446
`2c65cf6` → #452 `da047ad` → segfault zero-init `d68be3b`). Built by the
provisioning lane; independently re-verified by the research lane (w19:pK).

**Inner sha256s (research-lane re-verification):**

| file | sha256 |
|---|---|
| `usr/bin/lpac` | `0e5d5fa8780065cde41d218b9bf4d7c28140ec6cd472301f468cef261a69bbde` |
| `usr/lib/libeuicc.so.2` | `ac99f9f0cbd7fcdce2ac41c8f5080f0b0dc0f08ecf1addf6133fd72fbe5ec90b` |
| `usr/lib/lpac/driver/driver_apdu_at_csim.so` | `2e3f108c828d3f73dc5330e01de96cc01c483f222ccfce1d8ac2614b1da0b2aa` |
| `usr/lib/lpac/driver/driver_http_curl.so` | `0c249fb52565e7ce8630fea4fbcf9fce8e968ce9c8fe1ff34e0b9585af58403e4` |

**Four-point gate (both parties, first-hand):** F1 `0123456789ABCDEF` table
in `libeuicc.so.2`; F2 `timerfd_create` import in both AT drivers (structural
proof of #446's bounded deadlines — stock uses plain `poll`); F3
`LPAC_HTTP_SSL_VERIFY` string in `driver_http_curl.so`; F4 MIPS ELF +
NEEDED-closure zero-unresolved across 16 objects (bundled
libatomic/libgcc_s/libssl3/libcrypto3/libnghttp2; bundled openssl-flavored
libcurl replaces system libcurl at deploy — restore on the §F list).
Plugin discovery: `DT_RPATH=/usr/lib/lpac` + loader first-entry+`/driver`
derivation (source-verified at `euicc-driver-loader.c`).

## Files

- `card-error-path-exit255.json` / `-fixverify.json` — the segfault fix's
  behavioral proof: same wire flow (STORE DATA → `6D00`), clean error JSON,
  exit 255, no crash (x86, same commit chain).
- `segfault-backtrace-gdb.log`, `gdb3.log` — the original backtrace
  (`free(0x3)` in `es10c_ex_euiccinfo2_free` ← `applet_main` chip/info);
  basis of upstream lpac PR #467.
- `mock-transcript.jsonl` — `mock_modem.py --case-sensitive` rehearsal
  transcript (the Qualcomm-class lowercase-rejection fingerprint mode).
- `gsma-rsp2-root-ci1.pem` — GSMA RSP2 Root CI1 public cert (osmocom
  live-CI bundle extraction; SKID `81370f51…` matches the published KeyID;
  chain-validates the silent.link SM-DP+ TLS endpoint — `Verify return
  code: 0`). `ci-bundle.pem` — the full live-CI bundle it came from.

Artifact tarball itself: `/tmp/opencode/lpac-build/out/` (executing lane's
build tree) — rebuildable from branch `route1-gate2` via the lane build
script; the sha256 above is the identity of record.
