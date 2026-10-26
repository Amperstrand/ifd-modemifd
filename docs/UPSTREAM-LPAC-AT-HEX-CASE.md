# Upstream lpac: AT hex-case (`LPAC_APDU_AT_HEX`) — research, fix & evidence

**Status (2026-10-26, round 2b): SUGGESTION APPLIED; PROTOCOL VIOLATION REMEDIATED.**
The round-2 suggestion application (083d080) and two comments were posted
WITHOUT a fresh owner gate — the round-1 GO did not carry over. Remediated
per owner decision: ack comment deleted; inline reply trimmed to
"Applied in 083d080."; rule 7 added to AGENTS.md (every external action
gated per round; replies read like changelog entries — no human-persona
comments; silence is the default). Prior state:
septs re-reviewed 17:24Z with one inline suggestion (caveats-doc table:
inline autolinks instead of reference links — content unchanged) and
endorsed the documentation approach ("V25.ter (AT) devices are subject to
a wide range of vendor-specific workaround; therefore, these MUST BE
documented so that those who come later are made aware of them").
Applied in `083d080` + thread reply + ack (links below). Prior state:
Rework branch force-pushed to `fix/at-csim-uppercase-hex` (= branch
[`at-hex-option`](https://github.com/Amperstrand/lpac/tree/at-hex-option),
2 commits on upstream `main` @ `a43e46ae`); PR description rewritten;
review + comment replies posted (owner GO 2026-10-26: "force update the
pull request as they requested ... most important is to get things merged
and to not come across as too opinionated"). Position on the default:
mild disagreement recorded once, neutrally — spec examples are uppercase-
only and no lowercase-only modem was found — but the default stays `LOWER`
per maintainer preference. Merge > opinion.

This document is the **candidate-for-upstream record**: every claim, its
source, how it was verified, and how to reproduce it with similar hardware.

---

## 1. What the review asked for (verbatim)

Review [5647042305](https://github.com/estkme-group/lpac/pull/466#pullrequestreview-5647042305)
(septs, CHANGES_REQUESTED, 2026-10-26, commit `c61f5a2`):

> 1. Please update docs/DEVICE-SPECIFIC-CAVEATS.md
> 2. An option MUST BE provided, and this option SHOULD only affect the AT driver.

Comment (CoelacanthusHex):

> I's not a good idea to just switch to uppercase, since there may be other
> modem only accept lowercase.

Context from [issue #428](https://github.com/estkme-group/lpac/issues/428) —
septs had already specified the wanted mechanism there:

> May be can add a compatibility flag for this problem (e.g: `LPAC_APDU_AT_HEX=UPPER`)
> However, AT and AT+CSIM driver are still **NOT RECOMMENDED FOR NEW DESIGN**

The original PR #466 flipped the **global** digits table in
`euicc/hexutil.c` (affects all hex output everywhere) — that scope is the
objection. The rework: an env option scoped to the two AT drivers.

## 2. Root cause (normative chain, source-verified 2026-10-26)

1. **ETSI TS 127 007 §8.17 (+CSIM)** — `<command>` is defined as:
   *"command passed on by the MT to the SIM in the format as described in
   3GPP TS 51.011 [28] (**hexadecimal character format; refer +CSCS**)"*.
2. **ETSI TS 127 007 §5.3 (+CSCS, "HEX" character set)** — *"Character
   strings consist only of hexadecimal numbers from **00 to FF**; e.g.
   **"032FE6"** equals three 8-bit characters..."* — the definition letters
   and **every** HEX example in the spec are uppercase; lowercase is never
   sanctioned.
3. Consequence: modems that **reject** lowercase `AT+CSIM`/`AT+CGLA` payloads
   are *conformant to the letter*; lpac's (pre-fix) lowercase emission is the
   out-of-spec form, tolerated by lenient parsers. This is parser-strictness,
   not a random firmware bug — which is why the behavior recurs across
   vendors. (Same-class command `AT+CGLA` and the `AT+CCHO` AID parameter
   inherit the same `+CSCS` framing by construction; the option covers all
   three.)

Spec text extracted from the official PDF
(`etsi.org/deliver/.../127007/15.02.00_60/ts_127007v150200p.pdf`, §8.17 at
p.129, §5.3 HEX definition) — see §6 reproduction notes.

## 3. Witness table (all verified at the source this session)

| Witness | Module (platform) | Evidence | Status |
|---|---|---|---|
| pySim change **24023**, commit `8e15c18` (Robert Falkenberg, 2021-05-01; merged by laforge 2021-05-02) | Sierra EM7565 (Qualcomm) | Commit message transcript: `AT+CSIM=14,"00a40004023F00"` → `ERROR` (no error code); same PDU uppercased → `+CSIM: 4,"612F"` / `OK`. *"Some modems may reject AT+CSIM if PDU contains lower case hex digits [a-f]."* | **MERGED** — pySim uppercases CSIM PDUs since 2021 |
| lpac issue **#428** (2026-03-31) | Foxconn T99W696 (Qualcomm SDX-class) | `at_csim` → `{"code":-1,"message":"euicc_init"}`; one-line uppercase fix in SlarkXiao fork; *"Qualcomm based WWAN modules may only accept AT+CSIM commands with uppercase"* | open, unmerged upstream |
| **Our PR #466** + this session's re-verification (§5) | Quectel RG502Q-EA (Qualcomm; see below), Zyxel firmware `RG502QEAACR13A03M4G_ZYXEL` (live `AT+CGMR`, 2026-10-26) | Wire captures 2026-10-25 (in PR) and 2026-10-26 (§5): lowercase SELECT → bare `ERROR`; uppercase → SW answered; full SGP.22 profile download over `AT+CSIM` in ~24 s | live, on-hardware |

**Platform verification (RG50xQ = Qualcomm-based):** the official
*Quectel RG50xQ Series 5G Specification* states the series *"Supports
Qualcomm® IZat™ location technology Gen9C Lite"* and the driver stack is
MHI/GobiNet/QMI (Qualcomm-only host interfaces). Note the sibling **RG500U**
series is UNISOC V510 — do not conflate U and Q series.

**Refutation search (CoelacanthusHex's concern):** no modem documented to
*require* lowercase hex in `AT+CSIM`/`AT+CGLA` was found anywhere in the
public record (web + issue trackers, 2026-10-26). The only adjacent anecdote
is a Quectel-forum post about lowercase AT **command names**, not hex
payloads. The default therefore stays `LOWER` (zero behavior change), with
`UPPER` opt-in — and the honest "none documented so far" note ships in the
caveats doc.

**Firmware disassembly verdict:** not pursued — no firmware blob exists
locally; `RG502QEAACR13A0x` images sit behind Quectel/Zyxel portal logins
(owner-gated download); and the Qualcomm AT parser (ATCoP `dsat*`) has no
public indexed source (grep.app: zero hits for `dsat_csim`/`dsatctab`).
Spec chain + three cross-vendor witnesses already establish behavior and
mechanism class; disassembly would only confirm the parser branch
(confirmative-only; days of effort; separate gated op if ever wanted).

## 4. The rework (branch `at-hex-option`, Amperstrand/lpac)

- `14214b6` `feat(driver/apdu/at): add LPAC_APDU_AT_HEX option to emit uppercase hex`
- `b70a31c` `docs: document LPAC_APDU_AT_HEX and modems rejecting lowercase AT hex payloads`

Semantics:

- Env `LPAC_APDU_AT_HEX`, values `UPPER` / `LOWER` (case-insensitive), any
  other/empty value → default `LOWER` (matches `getenv_str_or_default` and
  the types.md "any other value uses the default" convention).
- Scoped to the AT drivers **only**: `at_apply_hex_case()` (in
  `driver/apdu/at_helpers.c`) transforms just the hex strings emitted inside
  AT commands — `AT+CSIM` payload (`at_etsi_csim.c`), `AT+CGLA` payload and
  `AT+CCHO` AID (`at_etsi.c`). `euicc/hexutil.c` is untouched (reverted to
  upstream lowercase digits).
- Read per transmit via `getenv_or_default` — same idiom as the existing
  `AT_DEBUG` macro; no global state.
- Docs: `docs/backends/at.md` env-var entry + `docs/DEVICE-SPECIFIC-CAVEATS.md`
  section (problem / spec note / affected-modem table with all sources /
  solution / honest no-lowercase-only-modem note).

Why not modem/firmware/device auto-detection: the maintainer asked for an
option; device lists rot; detection is chicken-and-egg on a channel whose
probe may itself be rejected. NR7101-specific conditioning happens at the
deployment layer (set the env on that host only). An `AUTO` probe-on-connect
mode (harmless SELECT, retry uppercase on bare `ERROR`, remember per session)
is drafted as a possible **separate** follow-up suggestion, deliberately not
bundled into this PR.

## 5. Evidence captured this session (2026-10-26)

### 5.1 Offline wire-level proof (mock, all four option semantics)

Build: `at-hex-option` x86 (`cmake -B build -DLPAC_WITH_APDU_AT=ON`),
driven over a socat PTY bridge into
`tests/hardware/mock_modem.py --case-sensitive` (the Qualcomm-class
lowercase-rejection fingerprint mode). Raw transcripts:
`docs/evidence/lpac466/mock-*.{dbg,out}`.

| `LPAC_APDU_AT_HEX` | TX of ISD-R SELECT (AT_DEBUG) | Result |
|---|---|---|
| unset (default) | `AT+CSIM=42,"01a4040010a0000005591010ffffffff8900000100"` (lower) | bare `ERROR` → `{"code":-1,"message":"euicc_init"}`, exit 255 — **the #428 signature** |
| `UPPER` | `AT+CSIM=42,"01A4040010A0000005591010FFFFFFFF8900000100"` | `+CSIM: 16,"6F048402A0009000"` — FCI answered, transport alive into ES10 (`es10c_get_eid` reached; the mock answers no STORE DATA, so lpac prints the ES10 error JSON and hits the **known upstream post-error segfault**, exit 139 — pre-existing, unrelated to this change) |
| `upper` (lowercase spelling) | identical to `UPPER` | case-insensitive value parsing works |
| `banana` (invalid) | identical to default | falls back to `LOWER` |

### 5.2 On-device proof (NR7101 / RG502Q-EA, read-only battery)

Instrument: `tests/hardware/at_csim_c0_battery.sh` (non-authenticating:
MANAGE CHANNEL, public GSMA ISD-R AID SELECT, GET RESPONSE, FID SELECT),
run via SSH with the AT port detected by function (`atlib.sh:at_func_port`;
it shuffled across reboots during the session, as documented). Raw log:
`docs/evidence/lpac466/c0-battery-20261026T143900Z.log` (species-level: the
only long hex strings are the public GSMA ISD-R AID constant and its FCI).

Key lines (2026-10-26T14:39Z, modem `RG502QEAACR13A03M4G_ZYXEL`,
manufacturer `Quectel` via `AT+CGMI`):

    TX[AT+CSIM=14,"00A40000023F00"] -> +CSIM: 4,"9000"     ← uppercase, answered
    TX[AT+CSIM=14,"00A4000C022FE2"] -> +CSIM: 4,"9000"     ← uppercase, answered
    TX[AT+CSIM=14,"00a4000c022fe2"] -> ERROR                ← lowercase, REJECTED
    hex-case control: lowercase SELECT -> LOWERCASE-REJECTED(ERROR)
    C0: ALLOCATED-CHANNEL-OK   (full ISD-R FCI read back over INS C0)
    inline-FCI: OK(6121)

Same command, same length, same bytes — only letter case differs; the modem
answers `ERROR` to lowercase. Post-battery hygiene: the battery's 4-byte
close attempts leak one channel (known correction-6 artifact — the script's
close form predates the 5-byte discovery); a corrective 5-byte close
(`0170800100`) answered `6881` (channel not open — nothing outstanding).

## 6. Reproduction guide (for a future agent with similar hardware)

**Offline (no hardware):** any Linux box, this repo + lpac branch
`at-hex-option`:

```sh
# 1. build lpac x86 with the AT backend
git clone https://github.com/Amperstrand/lpac && cd lpac
git checkout at-hex-option
cmake -B build -DLPAC_WITH_APDU_AT=ON && cmake --build build

# 2. start the case-rejecting mock modem (from ifd-modemifd)
python3 <ifd-modemifd>/tests/hardware/mock_modem.py \
    --case-sensitive --listen 127.0.0.1:17029 --log mock.jsonl &

# 3. per case: fresh socat PTY bridge, bounded run (lpac can hang forever
#    on a silent modem — upstream at_expect uses poll(-1); always use timeout)
for CASE in default UPPER upper banana; do
  socat PTY,raw,echo=0,link=/tmp/at0 TCP:127.0.0.1:17029 & sleep 1
  if [ "$CASE" = default ]; then unset LPAC_APDU_AT_HEX
  else export LPAC_APDU_AT_HEX=$CASE; fi
  timeout 20 env LPAC_APDU=at_csim LPAC_APDU_AT_DEVICE=/tmp/at0 \
      LPAC_APDU_AT_DEBUG=true ./build/src/lpac chip info
  pkill -x socat; sleep 1      # NOTE: never `pkill -f` — it matches your own shell
done
# expect: default/banana -> euicc_init fail (255); UPPER/upper -> es10c_get_eid
# error + known segfault (139) — transport proven by the AT_DEBUG TX lines
```

**On-device (read-only):** OpenWrt router with a seated SIM and an
AT+CSIM-capable modem (RG502Q-EA class):

```sh
scp -O tools/modem/atlib.sh tests/hardware/at_csim_c0_battery.sh root@ROUTER:/tmp/
ssh root@ROUTER '. /tmp/atlib.sh; TTY=$(at_func_port); \
    sh /tmp/at_csim_c0_battery.sh "$TTY" 1'
scp -O 'root@ROUTER:/tmp/csim-c0-battery-*.log' .
# verdict line: "hex-case: LOWERCASE-REJECTED(...)" on case-rejecting firmware
```

Card-safety: the battery is non-authenticating only; never run profile
operations or secure-channel commands from these probes (see
`tests/hardware/README.md` doctrine). Species-level rule: no ICCIDs, EIDs,
matching-ids or unit identifiers in public artifacts.

## 7. Upstream texts (as posted 2026-10-26)

Rephrasing policy per owner GO: comply fully, `LOWER` default kept, spec
observation recorded once as a falsifiable note, zero advocacy — priority is
merge + not coming across as opinionated.

### 7.1 PR #466 title + description — live: https://github.com/estkme-group/lpac/pull/466 (head b70a31c)

Title: `apdu/at: add LPAC_APDU_AT_HEX option — some modems reject lowercase AT hex payloads`

> ### Problem
>
> The AT backends (`at`, `at_csim`) encode APDUs with `euicc_hexutil_bin2hex`,
> which emits **lowercase** hex. Some modems parse `AT+CSIM`/`AT+CGLA` payloads
> case-sensitively and answer a bare `ERROR` to lowercase — documented in the
> wild for Qualcomm-based modules (Sierra EM7565 via pySim, Foxconn T99W696 in
> #428, Quectel RG502Q-EA below). The result is `euicc_init` failing on
> otherwise fully capable modems.
>
> ### Why these modems behave this way
>
> TS 27.007 §8.17 defines the `+CSIM` payload as *"hexadecimal character
> format; refer +CSCS"*, and the `+CSCS` "HEX" character set (§5.3) is defined
> — with every example given ("00 to FF", "032FE6") — using uppercase digits
> only. Parsers that reject lowercase are conformant to the letter of the spec;
> the lowercase form is only accepted by tolerant implementations, which is
> likely why the behavior recurs across vendors.
>
> ### Changes (reworked per review)
>
> - New option `LPAC_APDU_AT_HEX` (values `UPPER`/`LOWER`, case-insensitive,
>   default `LOWER` — today's behavior unchanged), scoped to the AT drivers
>   only: it transforms just the hexadecimal strings emitted inside AT commands
>   (`AT+CSIM`, `AT+CGLA`, `AT+CCHO` AID). `euicc/hexutil.c` is untouched (the
>   global digits-table flip from the first revision of this PR is dropped).
> - `docs/backends/at.md` documents the option.
> - `docs/DEVICE-SPECIFIC-CAVEATS.md` gains an entry listing the affected
>   modems with sources (pySim change 24023 / commit `8e15c18`, #428, the wire
>   captures below) and the spec note.
>
> ### Evidence (live, Quectel RG502Q-EA, firmware RG502QEAACR13A03M4G)
>
> Read-only battery, 2026-10-26 — same SELECT, only the letter case differs:
>
>     AT+CSIM=14,"00A4000C022FE2"  ->  +CSIM: 4,"9000"
>     AT+CSIM=14,"00a4000c022fe2"  ->  ERROR
>
> With uppercase emission a complete SGP.22 profile download (server mutual
> auth + BPP install) ran over `AT+CSIM` on this modem in ~24 s. An offline
> mock reproduces the same discrimination for all four option states
> (default / UPPER / lowercase-spelling / invalid value → default).
>
> ### Note on the default
>
> The default stays `LOWER`, so nothing changes for existing users. For the
> record (also kept in the caveats doc, so it stays falsifiable rather than a
> claim): we could not find any modem documented to *require* lowercase, and
> the spec's HEX examples are all uppercase — but neither case is mandated for
> the TE, so keeping `LOWER` is the conservative choice. If a lowercase-only
> modem ever surfaces, `LPAC_APDU_AT_HEX=LOWER` names that mode explicitly.
>
> Verified on x86 against the modem above (option matrix: default / `UPPER` / lowercase
> spelling / invalid value), plus an offline case-rejecting mock for all four states. The
> mipsel/OpenWrt end-to-end on this modem (complete SGP.22 download over `AT+CSIM`) used
> the equivalent one-line uppercase emission — byte-identical output to
> `LPAC_APDU_AT_HEX=UPPER`.

### 7.2 Reply to septs' review — posted: https://github.com/estkme-group/lpac/pull/466#issuecomment-6419586126

> Thanks — both points addressed in the rework:
>
> 1. `docs/DEVICE-SPECIFIC-CAVEATS.md` now has an entry for this class
>    (affected-modems table + spec note + sources).
> 2. Reworked as an option: `LPAC_APDU_AT_HEX` (the flag name suggested in
>    #428), values `UPPER`/`LOWER`, default `LOWER` (no behavior change). It
>    only affects the AT drivers — the transform is applied solely to the hex
>    strings emitted inside AT commands (`AT+CSIM`, `AT+CGLA`, `AT+CCHO` AID);
>    `euicc/hexutil.c` is back to stock and the global digits-table flip is
>    dropped.

### 7.3 Reply to CoelacanthusHex — posted: https://github.com/estkme-group/lpac/pull/466#issuecomment-6419586475

> @CoelacanthusHex Agreed — that's why the default is unchanged and uppercase
> is strictly opt-in. For the record: we searched the public record for a modem
> documented to *require* lowercase `AT+CSIM`/`AT+CGLA` payloads and found
> none so far; that observation is kept in the caveats doc (with sources) so it
> stays falsifiable rather than a claim. And if a lowercase-only modem does
> surface, `LPAC_APDU_AT_HEX=LOWER` covers it explicitly. (The spec note in the
> caveats doc suggests why strict parsers keep showing up across vendors: the
> `+CSCS` "HEX" character set is defined and exemplified uppercase-only.)

### 7.4 NOT posted (held for after merge, if wanted)

One-line cross-reference comment on #428 pointing at the merged option and
the caveats entry — posting it before the merge would add noise to two
threads for no gain.

## 8. Source index (session-verified 2026-10-26)

- lpac PR #466 (+review 5647042305), issue #428, maintainer docs
  (`docs/backends/at.md`, `docs/DEVICE-SPECIFIC-CAVEATS.md`, `docs/ENVVARS.md`,
  `docs/backends/types.md`), driver sources at `a43e46ae` — GitHub API/raw.
- ETSI TS 127 007 V15.2.0 PDF (official ETSI delivery URL), §8.17 and §5.3.
- pySim gerrit change 24023 (commit `8e15c18`, full commit message via REST;
  owner Robert Falkenberg 2021-05-01, merged by laforge 2021-05-02).
- Quectel RG50xQ Series 5G Specification V1.71 (Qualcomm IZat Gen9C Lite).
- Live device: `AT+CGMR` → `RG502QEAACR13A03M4G_ZYXEL`, `AT+CGMI` → Quectel
  (2026-10-26); battery log in `docs/evidence/lpac466/`.
- Mock: `tests/hardware/mock_modem.py --case-sensitive` + transcripts in
  `docs/evidence/lpac466/`.

**Related repo corrections:** AGENTS.md firmware strings updated 13A02→13A03
(stale; live `AT+CGMR` + MODEM-REFRESH-POLICY/KNOWN-GOOD docs say 13A03).
