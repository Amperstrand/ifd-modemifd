# DRAFT — owner-gated, human files. Do not auto-publish.

Target: openwrt/packages — (a) a comment on #24963 attaching the trust-anchor
half of the story; (b) optionally a new package proposal `gsma-rsp-certs`.

## Proposed comment on #24963 (mbedtls/lpac TLS failures)

Title context: "lpac: OpenWRT libcurl & mbedtls not working with lpac"

---

Two separate problems are being conflated in this class of failures. The
mbedtls handshake incompatibility (this issue) is one half. The other half is
a **missing trust anchor**: consumer eSIM SM-DP+ endpoints present TLS
certificates that chain to the **GSMA CI roots, not to WebPKI**. On OpenWrt,
`ca-bundle` alone can never validate an SM-DP+ server — with verification
on, every profile download fails; the usual workaround (verification off) is
exactly what nobody should ship.

What we did (species-level, bench-verified with the osmocom "known test
profiles" Speedtest profile, endpoint class rsp.truphone.com/1GLOBAL):
augment the CA file with the five **GSMA live eSIM CIs** — GSM Association
RSP2 Root CI1 (DigiCert, key ID 81370f…, valid to 2052), OISTE GSMA CI G1,
Entrust eSIM CA, MC4 OT ROOT CI v1, SubMan V4.2 CI (Google Pixel) — and keep
verification on. Source of the certs: the GSMA-published eSIM certificate
set, as mirrored by the osmocom eUICC manual live-CI bundle (itself an
extract of the chromium `hermes` cert set). Download works with zero TLS
errors once the CI roots are present.

Proposal: a small `gsma-rsp-certs` package (or a `ca-certificates` variant)
shipping the live CI roots to `/etc/ssl/certs`, sourced from the GSMA
certificate set with a checksum-pinned update path. Every OpenWrt LPA user
(currently: the lpac feed package with uqmi/at/mbim backends) hits this
wall; today everyone rediscovers it the hard way.

---

## Filing checklist (owner)

- [ ] Re-verify #24963 is still open and unmuted at file time
- [ ] Species-level only: no bench identifiers, no EID/ICCID, no unit data
- [ ] If the new-package proposal is filed separately, cross-link both
- [ ] Attach or link the osmocom CI bundle page, not our private mirrors
