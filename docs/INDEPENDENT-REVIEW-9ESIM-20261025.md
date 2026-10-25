# Independent Architecture Review — 9eSIM Provisioning & Modem Reader Architecture

**Reviewer**: independent architecture-review session (Herdr pane `w19:pJ`),
commissioned 2026-10-25. **Mandate**: evaluate the near-term 9eSIM
provisioning route and the longer-term PC/SC reader architecture from
evidence, coordinating with — but not deferring to — the working lanes.
No hardware operations were performed by this review; every conclusion
below rests on archived artifacts, source code, or upstream references.

**Confidence taxonomy** used throughout: **Verified** (reproduced on the
relevant hardware or supported by authoritative implementation source),
**Strongly supported** (multiple credible sources, target-hardware
validation incomplete), **Hypothesis** (plausible, not demonstrated),
**Contradicted** (evidence disproves), **Unknown** (insufficient evidence).

Species-level record, matching the repo's public-repo policy: no unit
identifiers, EIDs, ICCIDs, IMEIs, matching-ids, order tokens, bench
addresses, or raw APDU payloads.

---

## 0. Executive verdict

1. **The GET RESPONSE experiment was invalid.** Every "GET RESPONSE"
   exchange in the 2026-10-24/25 CSIM windows used instruction byte
   `2C` (ISO 7816-4 RESET RETRY COUNTER), never `C0` (GET RESPONSE).
   The `6D00` responses therefore say nothing about GET RESPONSE
   capability. The owning lane has independently confirmed this and
   landed correction commit `a46625c` voiding the evidence. **Verified.**
2. The headline theory — *an enabled profile structurally prevents any
   coherent LPA/ISD-R session over AT+CSIM* — loses its central pillar
   and is downgraded to **Hypothesis**. A simpler, unexcluded
   explanation exists for the observed lpac failures (self-induced
   logical-channel exhaustion; §A.3).
3. The MBIM pivot for the **imminent** provisioning operation survives
   this correction on independent legs — but rests on an **Unknown**:
   MBIM UICC low-level access has never been exercised on the RG502Q-EA
   (wwand's MBIM UICC evidence is wire-level against libmbim with no
   eUICC-capable MBIM modem on hand). The runbook's read-only gate is
   therefore mandatory, not optional. Two hardening amendments are
   recommended before any owner GO (§D).
4. PR #30 (vpcd) is **not ready** to be load-bearing for anything
   time-critical: five first-hand-verified defects (§A.6), one of which
   (synthetic `6F00` on delivery-unknown APDUs) violates the project's
   own v3.4 no-resend doctrine. It should remain a separate, deferred
   workstream.
5. **Recommendation shape**: no re-pivot before the window (schedule
   risk for zero gain); MBIM path behind owner gates with the §D
   amendments; corrected CSIM probe battery as the immediate post-window
   research item; PR #30 deferred and re-scoped (§E, §F).
6. **Late pivot (2026-10-25 evening), reviewed**: with the silent.link
   credential in hand and the MBIM gates (D2–D6) unclosed, the working
   lane selected the **physical-reader path for tonight's window**
   (ccid-firmware-rs issue #459). This review returned **AMEND —
   none blocking; path choice endorsed** (Option D is the only
   class-proven transport available tonight without gambling the
   one-shot credential on an unproven one). Amendments sent: corrected
   "at_csim DEAD" wording; free-NVM + notification-queue-branch +
   census-EID-provenance preflight adds; explicit enable-failure parking
   state; notification-failure-after-enable marked non-fatal;
   second-reader-cycle rollback named; no-deletion of the old profile;
   EID kept out of public trackers. The on-device MBIM path (runbook
   v3 + §D gates) remains the appliance goal for after this window.

---

## A. Research findings

### A.1 The GET RESPONSE experiment — verdict: INVALID

Evidence chain (all artifacts inspected first-hand this session):

| # | Fact | Artifact |
|---|---|---|
| 1 | The probe constructor builds the "GET RESPONSE" with INS `2C`: `GETR="AT+CSIM=10,\"${CLA}2C000021\""` — in both the main round and the determinism re-run | `slot2-round4.sh` lines 58, 69 (lane-side archive, `/tmp/opencode/esim-probe/`) |
| 2 | That script also encodes the channel CLA as `printf '4%X'` — the 7816-4 form for channels 5+, wrong for channels 1–3 | same, line 56 |
| 3 | The archived round-4 run never reached the `2C` exchange at all: the channel open returned `6A81` (exhausted), so the `if [ -n "$CH" ]` guard skipped the entire SELECT/GET RESPONSE block | `slot2-round4-20261024T220340Z.log` |
| 4 | No `C0` INS byte appears in any surviving probe script or archived log; the only `+CSIM` exchanges on record are SELECT/MANAGE-CHANNEL/B0-class | grep across lane archives (confirmed independently by two lanes) |
| 5 | The `6D00` row originated from window tests *intended* as C0 discrimination but instrumented with the same `2C` constructor — the owning lane's session record now states "no genuine C0 data exists" | `2026-10-25-esim-card5-ondevice-transport.md` (corrected in place); correction commit `a46625c` on `findings/csim-session-state` |
| 6 | ISO 7816-4: GET RESPONSE is INS `C0`; `2C` is RESET RETRY COUNTER (a security-management instruction) | standard; also noted by both reviewing lanes |

Consequences:

- A `6D00` (instruction not supported) response to RESET RETRY COUNTER
  on an eUICC is unsurprising and carries **zero** information about
  GET RESPONSE transport.
- The finding-doc row "GET RESPONSE (`0n 2C 00 00 21`) → 6D00, both
  ch1- and ch2-class CLAs, 1.5 s and 8 s gaps" is **void** and has been
  struck by `a46625c`.
- The related "pacing" retraction (correction 3 of the original doc) is
  moot — the paced-vs-unpaced comparison compared two flavors of a
  wrong instruction.
- Safety note (standing): `2C` must never be used as a diagnostic probe
  again — RESET RETRY COUNTER is security-class. The corrected battery
  (§E, phase 1) uses `C0` only.

### A.2 Evidence that survives (INS-independent, log-backed)

From the stamped, archived lane evidence (`slot2-probe-…Z.log`,
`slot2-round4-…Z.log`, `slot2-cfun-…Z.log`):

| Observation | Status | Notes |
|---|---|---|
| Basic-channel SELECT MF by FID → `9000`, deterministic | Verified | genuine card access |
| Basic-channel SELECT EF (P2=0C) + READ BINARY → real EF data (ICCID read back byte-exact) | Verified | proves the CSIM path reaches the *correct* (active-slot) card |
| MANAGE CHANNEL open allocates channels (`02`,`03` → `9000`) until exhaustion (`6A81` after ~2–3 opens) | Verified | exhaustion is the *card's* expected answer when channels are leaked |
| MANAGE CHANNEL close (basic-close and channel-form) → AT-level `ERROR`, always | Verified | modem policy refusal, not a card SW; the leak driver |
| CFUN recycle clears leaked channels (post-recycle open returns a fresh channel) | Verified | the recovery knob |
| ISD-R SELECT (CLA=`01`, P1=04, P2=0C, no Le) → `61 21` — *including on a round where the channel open had just been refused* | Verified (observation) | the `61 21` is decoupled from channel accounting; interpretation open (§A.4) |
| ISD-R SELECT inline-FCI form (P2=00, Le=00) → `6A82` | Verified (observation) | same AID, different P2, different answer — the pair is internally inconsistent with a plain card-side SELECT handler |
| lpac (at_csim) `chip info`/`profile list`/`notification list` → `euicc_init` failure ×3 | Verified (fact), Unknown (mechanism) | no wire-level capture of what lpac sent; leading hypothesis §A.3 |
| `AT+CFUN=0` → SIM invalid (`+CME ERROR` on CPIN/CSIM) | Verified | matches hardware design doc |
| `AT+QESIM` operations vestigial on this firmware generation; `lpa_enable` gating + documented wedge incident | Strongly supported | field reports + vendor answer + lane incident record |
| CLA `0x4X` probes reached channels 5–7, not 1–3 (original encoding error) | Verified | corrected in doc §3.2 at the time; the surviving `61 21` row's CLA provenance flaw is now flagged by `a46625c` |

### A.3 Why lpac initialization failed — leading hypothesis (new)

Upstream lpac `driver/apdu/at_etsi_csim.c` (pinned ref `a43e46ae`),
read this session:

- `logic_channel_open` = MANAGE CHANNEL open (expects `xx 90 00`) then
  SELECT by AID with **P2=`00`, no Le** (case 3), and **accepts SW
  `61xx` as success** — so even the observed `61 21` answer would not
  by itself fail lpac's open.
- `logic_channel_close` sends the **basic-channel close form**
  `00 70 80 <ch>` — exactly the form this modem AT-refuses. Every lpac
  at_csim session therefore **leaks its channel**.
- The windows' own probes had already demonstrated: two leaked channels
  → next open returns `6A81` → lpac's open fails its `9000` check →
  `euicc_init` fails.

**Hypothesis (leading, unexcluded, cheap to test)**: the three lpac
failures were self-induced channel exhaustion — probes leaked channels,
and/or each lpac run leaked one more — in a window with no intervening
CFUN recycle. No enabled-profile mechanism is required to explain them.
Discriminating test: CFUN recycle → immediate `lpac chip info` with
APDU debug capture (post-window item, §E). Status of the original
"consistent with the GET RESPONSE result" linkage: **Contradicted**
(the referenced result is void).

### A.4 The `61 21` / `6A82` anomaly and the surviving mechanism candidates

The two ISD-R SELECT answers on record are mutually inconsistent with a
straightforward card SELECT handler (P2=0C "no data" answered with
"bytes pending"; P2=00 "data wanted" answered "not found"), and the
`61 21` arrives even when no channel exists. Candidate mechanisms, all
still live, none proven:

1. **Modem-intercepted context** — the modem services AID SELECTs from
   its own managed session (original doc §3). Explains decoupling;
   requires modem-side AID matching.
2. **T=0 procedure surfacing/absorption** — if the MT↔card link is T=0
   (ATR measured: T=0 offered first, T=1 also offered), `61xx` handling
   belongs to the modem; whether it auto-follows or surfaces `61xx` to
   the host is firmware-specific. Notably, the v4.0 hardware evidence
   records "logical-channel lifecycle … clean, **FCI answered**" through
   AT+CSIM while the shipping driver contains **no 61xx/GET RESPONSE
   logic at all** (`src/reader.rs::transmit_with_timeout` is a raw
   pass-through) — i.e. on the blank card, response retrieval worked
   with the *modem* owning the procedure. Status: Strongly supported
   that the modem can own it; Unknown whether it does so uniformly.
3. **Channel-exhaustion aliasing** — the `61 21`-after-refused-open
   round ran while all channels were leaked; a modem mediating onto its
   own session would look exactly like this. Sub-case of (1).

The original §3 single-mechanism claim ("the baseband's enabled-profile
session structurally prevents LPA access") is a **Hypothesis** until a
corrected battery discriminates.

### A.5 QMI/MBIM feasibility — verified facts and the one Unknown that matters

**QMI (native)**

- Feed libqmi 1.36.0 (OpenWrt packages, mipsel_24kc) ships with the UIM
  APDU surface **compiled out**: `qmi-uim.h` in the cross-build SDK
  shows `HAVE_QMI_MESSAGE_UIM_SEND_APDU`, `…UIM_LOGICAL_CHANNEL`,
  `…UIM_OPEN_LOGICAL_CHANNEL` all commented. lpac's `qmi` backend
  cannot build against it. **Verified** (header inspected in the lane's
  SDK staging dir). Consequence: the QMI route needs a self-built
  libqmi or a different transport — out of scope for the imminent
  window.
- wwand (upstream, `docs/STATUS.md` state 2026-10-24), measured on the
  **exact modem class** (NR7101/RG502Q-EA, 2026-09-26): both physical
  slots map to **logical slot 1**; `SEND_APDU` on logical slot 2 is
  refused `NOT_SUPPORTED (94)`; a logical channel "opened on slot 2"
  lands on the active card; `AT+QUIMSLOT` is the only physical-slot
  switch. **Verified (by wwand, on our exact hardware class).**
  Consequence: no QMI `UIM_SLOT=2` addressing scheme can reach "the
  slot-2 card" — only the *active* card is ever reachable, and here the
  active card already is the eUICC. Any plan that assumes per-slot QMI
  addressing on this modem is wrong.

**MBIM (lpac `mbim` backend, upstream source `a43e46ae`, read this session)**

- Open-channel = atomic open+SELECT with P2=`0C` — **no host-side GET
  RESPONSE is ever needed**; the modem constructs channel CLAs itself.
  This is structurally immune to the entire §A.1/A.4 APDU-encoding
  problem class. **Verified (source).**
- Disconnect closes a leaked channel explicitly; STORE DATA gets Le
  appended. **Verified (source).**
- Timeouts (mbim_helpers, source): 15 s open / 10 s per command / 20 s
  close. The lane's window artifact raises per-command to 30 s (one-line
  patch, build-script-baked; sha256 `19d6f87d…aa94`; fallback stock
  build `7307e038…d45b`; both hashes re-verified on disk this session).
- **Slot-selection semantics — the sharpest unappreciated risk**: on
  connect, lpac *queries* the MBIM device slot mapping and **writes a
  new mapping if it does not match `UIM_SLOT`** (`mbim.c
  select_sim_slot`). The env is 1-based (default **1**). On this
  dual-slot, single-standby modem, forgetting `UIM_SLOT=2` does not
  merely "read the wrong card" — it can **actively rewrite the active
  physical slot** and wait on the other card's readiness. The runbook
  sets `UIM_SLOT=2` (correct intent), but what the RG502Q-EA *reports*
  for the mapping — and therefore whether lpac's query matches and skips
  the write — is **Unknown** (wwand has no MBIM data for this modem;
  their MBIM rows are other modems). Mitigation that removes the write
  path entirely: `LPAC_APDU_MBIM_SKIP_SLOT_MAPPING=1` — the active card
  already is the target, so no mapping change is ever needed. **This
  review recommends the skip-first protocol (§D gate D2).**
- **MBIM UICC on RG502Q-EA: Unknown.** wwand's own status: MBIM UICC +
  lpac is wire-verified against libmbim 1.32 but *not* end-to-end — "no
  eUICC-capable MBIM modem is on hand". wwand's transport priority
  (MBIM → QMI → AT) concurs with the pivot direction. Composition:
  ECM exposes no `/dev/cdc-wdm*`; usbnet=2 (MBIM) does; the unit's
  factory/reference composition was QMI-class (cdc-wdm present) —
  i.e. the modem has run non-ECM compositions before. Registration
  side-effects of composition switches on this vendor family are
  cautioned by wwand's LTE3301-PLUS/EG06-E experience (different modem;
  cautionary only).

**Historical AT+CSIM success (v4.0)** remains Verified and gains
importance: full SGP.22 download+enable+notification, one shot, ~60 s,
zero transport errors, through the same modem family — on a blank card.
It is the standing proof that this modem's AT+CSIM can carry a complete
LPA session *under some session state*, and it featured working response
retrieval with no host-side GET RESPONSE (§A.4). Blank-card success must
not be generalized to the enabled-profile card — but it cannot be
dismissed either, and until the corrected battery runs, "AT+CSIM cannot
provision an enabled-profile card" is unproven in **both** directions.

### A.6 PR #30 (vpcd) — first-hand audit

Diff base `3d850af`…`90f1e3f`; all findings verified against the code
this session, plus upstream `frankmorgner/vsmartcard`
`virtualsmartcard/src/vpcd/vpcd.h` (fetched upstream):

```c
#define VPCD_CTRL_OFF   0
#define VPCD_CTRL_ON    1
#define VPCD_CTRL_RESET 2
#define VPCD_CTRL_ATR   4
```

| # | Finding | Evidence | Severity |
|---|---|---|---|
| 1 | **Transport errors answered as card responses**: `src/agent.rs` maps `transmit_with_timeout(...).Err(())` — which per `AtOutcome` means *delivery unknown or provably failed* — to synthetic `SW 6F00`, keeping the vpcd session alive. This collapses NotSent/Ambiguous into an apparent card refusal and violates the v3.4 doctrine the same crate enforces on its pcscd path (`NotSent/Ambiguous → IFD_COMMUNICATION_ERROR, never a synthetic SW`). A PC/SC client may treat `6F00` as a card-level failure and retry a possibly-executed state-changing APDU. | `src/agent.rs` `Frame::Apdu` arm vs `src/reader.rs::transmit_with_timeout` | **High** for any state-changing use |
| 2 | **Median-based deadline**: `APDU_TIMEOUT` = 1.5 s, sized from the 400 ms p50 and the patched driver's 2 s receive window. eUICC final-install APDU latency is unmeasured (the MBIM lane independently chose 30 s for exactly this reason). The full chain (app → pcscd → vpcd window → agent → modem → card) needs coordinated, operation-aware deadlines. | `src/agent.rs` const + comment | High for provisioning-class use |
| 3 | **Control labels inverted vs upstream**: PR defines `CTRL_ATR=0x02, CTRL_RESET=0x04`; upstream is `RESET=2, ATR=4`. Behavior is coincidentally compatible (both arms answer ATR, and both a RESET and an ATR request are answered with an ATR), but `ON (0x01)` falls into "unknown control ignored", the unit test asserts the swapped names as "reference", and the naming will bite anyone diffing against upstream. | `src/vpcd.rs` vs upstream `vpcd.h`; `agent/tests/vpcd.rs` | Medium (correctness hazard, latent) |
| 4 | **RESET answered with the canned fake ATR**: a router agent cannot cold-reset the seated SIM; answering RESET with "fresh ATR" asserts a session-state change that never happened. Honest behavior is to model what the modem can do (e.g. CFUN recycle under lane lock) or refuse. | `src/agent.rs` control arm | Medium |
| 5 | **No per-tty ownership**: `open_serial` (`src/transport.rs`) runs `stty` and opens the tty with **no flock and no modem-lane participation** — reintroducing the byte-theft class the project already burned on 2026-10-24 and violating PR #30's own architecture doc requirement ("agent participates in the router-side modem-lane lock and holds the per-tty flock for its lifetime", `VPCD-ARCHITECTURE.md` lines 55–57). The ssh transport's hardening (flock, PDEATHSIG, self-reaping relay) does not carry over. | `src/transport.rs` `open_serial` | **High** for router-side deployment |

Additionally: the PR-review comment's point 5 ("the agent's transport
is dead for the enabled-profile eUICC case") rests in part on the now-
voided GET RESPONSE evidence; the AT transport is *unreliable* for
multi-exchange LPA use on this firmware while closes are refused
(channel leak per session), but "dead" is overclaimed until the
corrected battery runs. The comment needs a correction note when the
PR is next touched.

### A.7 Cross-document discrepancy registry (open items)

| # | Discrepancy | State |
|---|---|---|
| 1 | README modem-finding: "READ BINARY dropped on the basic channel" vs round-4 log: basic-channel READ BINARY returned real EF data (`9000` + ICCID bytes) | Both measured, different dates/units or session states; reconcile on next hardware lane round. Not blocking the MBIM path. |
| 2 | README: "CCHO/CCHC work; CGLA swallowed" vs runbook v2's "CCHO/CCHC disabled" | The runbook claim had no instrumented basis and is gone from the corrected doc (a46625c survivors-only rewording). README's measured claim stands. |
| 3 | The relayed "C0/E2 blocked" allowlist verdict | **No artifact exists anywhere** (scripts, logs, session text — searched by two lanes independently). Treated as unproven rumor; struck from consideration. If the E2 (STORE DATA) half were ever substantiated it would matter — but note v4.0 ran 150× STORE DATA on a logical channel successfully, so the burden of proof is high. |
| 4 | Lane summary phrase "MANAGE CHANNEL open → 6A81: the logical-channel path is refused at the modem" | Imprecise: opens succeed until card-side exhaustion; it is *closes* the modem refuses. Corrected in the record here; no doc change needed beyond a46625c. |

---

## B. Cross-agent review record (Herdr coordination)

Participating sessions (workspace `w19`, 2026-10-25):

| Pane | Session | Role in this review |
|---|---|---|
| `w19:pJ` | this session | independent reviewer; no hardware authority |
| `w19:p4` | ifd-modemifd audit session | audited the archived evidence on request; independently confirmed 2C constructor + zero C0 + missing wire provenance; retracted its second-hand "C0 blocked" relay |
| `w19:pH` | architecture lane (QMI/MBIM research, artifacts, PR #30 comment) | responded with CONCUR-with-extension (CLA=01 provenance flaw; round-4 never reached 2C); defended the MBIM pivot on independent legs; proposed the corrected C0 battery |
| `w19:pF` | owning lane (spare-eSIM provisioning window; runbook + findings doc) | landed correction commit `a46625c`; corrected the session record; accepted the corrected C0 battery as a post-window roadmap item; owner gates unchanged |

Dispute log:

1. **"Does the CSIM correction re-open the near-term route decision?"**
   Reviewer initially treated the AT route as possibly revived; architecture
   lane pushed back (MBIM legs are independent of the CSIM theory;
   re-pivoting buys schedule risk for zero gain). **Resolution: concur —
   no re-pivot before the window.** Recorded agreement.
2. **"Is the enabled-profile mechanism claim still standing?"** All
   lanes now agree: downgraded to hypothesis; survivors enumerated
   (§A.2). Disagreement closed by evidence, not seniority.
3. **"Opens refused vs closes refused"** (§A.7 item 4) — precision fix
   accepted.
4. **Open, unresolved**: MBIM slot-mapping write behavior on RG502Q-EA
   (this review's §A.5 risk); README-vs-round-4 basic-channel B0
   divergence (§A.7 item 1). Both have named discriminating checks
   (§D gate D2; next hardware lane round).

Adversarial review of this report was requested from all lanes
(announcement + findings distributed through Herdr); the architecture
lane's challenge round is incorporated above. Lanes were asked to
identify: an overlooked failure mode (→ slot-mapping write risk, added
to §D), a simpler architecture (→ none offered beyond the standing
options), a contradictory observation (→ §A.7 registry), a
documentation-not-measurement assumption (→ the voided GET RESPONSE row
itself and the "C0/E2" rumor), and a recovery-plan failure mode (→
EG06-E MBIM registration caution, noted in §A.5).

**Final pre-gate deliveries from the architecture lane** (coordinator
session w19:pF exited; artifacts in shared spaces), all endorsed by
this review:

1. **PR #30 disposition memo** (pull/30 comment 6395364117) — decision
   tree branched on the corrected C0 battery: if genuine C0 works, PR
   #30 revivable with fixes 1–4 (§A.6) and topology re-argued; if C0
   fails cleanly, transport-swap (only if the owner picks the
   vpcd/Option-C reader line) or close-and-salvage. Codec + mock
   battery are durable assets either way. No merge before issue #32 and
   the C-vs-F decision. Sound — matches §F.
2. **PR #33 Option-F spike protocol appendix** (commit `ade4c77`) —
   wwand from its feed on a sacrificial NR7101; four phases ending in
   the C-vs-F decision matrix; sacrificial-unit gates + rollback +
   1-day timebox. Sound — sacrificial-first ordering matches §E phase 2.
3. **Quectel APDU_decrypt ticket** (drafted for owner send,
   `~/.esim-reseller/night-20261024/QUECTEL-APDU-DECRYPT-TICKET.md`) —
   five questions including baseline availability vs the R01-only field
   evidence; vendor-hygiene: no probe-defect history disclosed; owner
   fills the exact ATI string. Sound.

Disagreement ledger at gate time: **zero unresolved entries** (§B items
1–3 closed by evidence; §A.7 registry items are open *questions* with
named discriminating checks, not inter-lane disagreements).

---

## C. Architecture decision matrix

Confidence column reflects the evidence state of **2026-10-25**, after
correction `a46625c`.

| Option | Near-term provisioning | Long-term PC/SC reader | Key risks | Confidence |
|---|---|---|---|---|
| **A. v4.1 AT+CSIM/PCSC (existing driver)** | Viable *if* the corrected battery revives coherent ISD-R sessions on an enabled-profile card; lpac pcsc path previously provisioned this exact card class (blank) | Proven generic reader for blank/generic cards; channel-leak-per-session makes multi-exchange LPA use fragile on this firmware | Channel-close refusal (leak); unexplained lpac ×3; `6121` anomaly unresolved | AT transport works at filesystem level: **Verified**. LPA-session revival: **Hypothesis** (battery pending) |
| **B. lpac over QMI UIM** | Dead on this feed (libqmi UIM APIs compiled out — **Verified**); a self-built libqmi is a new build+deploy surface for zero near-term gain | Attractive longer-term (vendor's own channel; wwand-proven on exact modem for read-class) — but note slot semantics: only the active card is addressable (fine — target is the active card) | Build complexity; modem-manager contention; slot semantics often misunderstood | Feasibility on exact modem: **Verified (read-class, by wwand)**. lpac-over-QMI on router: **Unknown** |
| **C. lpac over MBIM UICC** | **Selected by the working lanes** (runbook v3). Structurally immune to the CSIM encoding problem class; artifacts built+hashed; timeouts sane | Could later back the vpcd agent as a pluggable backend | **MBIM UICC on RG502Q-EA unmeasured (Unknown)**; slot-mapping *write* on mismatch (mitigated by SKIP_SLOT_MAPPING); composition switch + reboot (~2 min down, recovery path documented); no sacrificial RG502Q exists — first exercise is on the production unit, read-only gate first | Backend source facts: **Verified**. On-target behavior: **Unknown → gate D2/D3 exists to decide it** |
| **D. Physical external reader** | Last-resort fallback; staged (reader + triple-gated scripts on the bench host); requires card reseat, off-product workflow | Not a reader architecture for the appliance goal | Physical handling of the production eUICC; workflow discipline | Class-proven: **Verified** (prior successful provisioning class) |
| **E. PR #30 vpcd + AT+CSIM** | Not a candidate (§A.6 defects; transport fragility) | Deferred; needs all five fixes + re-scope | Synthetic-6F00 retries on ambiguous APDUs; timeouts; ownership | Defects: **Verified** |
| **F. vpcd + native QMI/MBIM backend** | No (not built) | The credible long-term shape *if* a general PC/SC surface is still wanted after Option C proves the direct-lpac path — vpcd's value is PC/SC-app compatibility, which direct lpac doesn't need | Double protocol translation (PC/SC↔vpcd↔MBIM↔APDU), timeout chain, ownership; every layer re-verified | **Hypothesis** |
| **G. Hybrid (C now, A/F research in parallel)** | The operative plan: runbook v3 for the imminent window; corrected battery + PR #30 disposition as parallel workstreams | Combines the fastest validated path with evidence-driven medium-term decisioning | Coordination overhead (managed via Herdr) | **Recommended** |

---

## D. 9eSIM readiness assessment — measurable go/no-go gates

These gates amend (do not replace) runbook v3. No gate below authorizes
hardware action; they define what must be true before the owner is
asked for GO.

- **D1 — Evidence hygiene (done)**: correction `a46625c` merged; no
  decision may cite the voided GET RESPONSE row or the "C0/E2" rumor.
- **D2 — Slot-mapping safety (new, blocking)**: the first lpac MBIM
  invocation on the router must run with
  `LPAC_APDU_MBIM_SKIP_SLOT_MAPPING=1` (the active card already is the
  target; eliminates the mapping-*write* class entirely). Separately,
  capture the device's *query-only* slot-mapping report (read-only,
  e.g. mbim-query class tool or a one-line lpac debug run) and record
  what the RG502Q-EA reports — this converts §A.5's Unknown into data
  without any write. Only if skipping is demonstrably broken may the
  `UIM_SLOT=2` mapping path be considered, and then only after the
  query shows the expected current mapping.
- **D3 — Transport gate (runbook step 2, endorsed with amendments)**:
  `chip info` must return an EID (read-only); soak chip×5/list×3;
  abort on any anomaly. Add: capture per-command latency from the soak
  to bound the BPP-install duration assumption feeding the 30 s
  timeout choice.
- **D4 — Artifact integrity (runbook PENDING item, blocking)**: the
  lane's independent re-verification of the mbim30 tarball
  (sha256 `19d6f87d…aa94`, pair-integrity vs stock `7307e038…d45b`)
  must be completed and recorded before deploy. (Reviewer re-verified
  both hashes on disk this session; the lane's ELF-level verification
  stands unreviewed by a second lane.)
- **D5 — Connectivity independence (blocking)**: confirm the router's
  non-SIM uplink carries the whole window (download does not depend on
  the SIM), including the ~2 min modem-absence window during the
  composition reboot, with the tollgate failover path exercised or its
  skip formally owner-accepted.
- **D6 — Recovery drill on paper (blocking)**: the composition revert
  (`usbnet=1` + reboot) must be written as a standalone runbook page
  executable by a second operator without context; the modem-wedge
  recovery (power cycle path) must be identified for the NR7101.
- **D7 — One-shot discipline (standing)**: order re-check immediately
  pre-window; no retry of a failed download without diagnosis;
  matching-id status checked before any re-attempt; TLS verification
  never weakened (GSMA root bundle present, fingerprint-verified —
  confirmed in the lane archive).
- **D8 — Profile preservation (the ordering question)**: the runbook's
  demoted-disable sequence is **endorsed**: download is ISD-R-scoped
  and additive; the existing enabled profile is touched only at the
  enable/displacement step, with re-enable of the old profile as the
  documented rollback. The pre-correction "disable first" requirement
  was an artifact of the blocked AT transport, not a card property.
  Capacity gate: free NVM read at step 2 must show sufficient space
  for one more profile before the download proceeds.

**Verdict**: the window is **NO-GO until D2, D4, D5, D6 are closed**
(all closable without touching the card); then owner GO/NO-GO decides.

### Provisioning-readiness verdict (2026-10-25, owner question: "are we
ready — one shot")

**Not yet — by design.** Three gates stand between today and the
one-shot, none touching the credential:

- **Gate 1 — the read-only window** (R1a/R1b/R2 + R-½; staged,
  runbook-ready). Its outcome selects the provisioning path: an
  AT-revival (R1b EID) means no composition switch at all; otherwise
  Route-1 QMI composition (uqmi artifact) with MBIM fallback.
- **Gate 2 — the hardened download artifact**: pin + #452 (TLS
  verification ON, GSMA-augmented trust store) **+ a fix for the
  post-error segfault** — a crash on a mid-download card-error SW would
  skip lpac's cancel-session and leave server-side state dangling with
  the matching-id consumed: precisely the wrong bug to meet during the
  one-shot. One build, fingerprinted like atcsimUC.
- **Gate 3 — a reusable-credential rehearsal download** (strongly
  recommended, owner-gated write): freely-downloadable test profiles
  exist (osmocom eUICC manual "Known Test Profiles": BetterRoaming /
  Speedtest via `rsp.truphone.com`, "any number of times … to any
  number of eUICCs", valid for GSMA-Live-CI cards such as the 9eSIM —
  verified this session). One test download through the exact window
  machinery (same build, same transport, same TLS leg, full BPP
  install + notification cycle, then `profile delete` cleanup after
  free-NVM gate) converts the real download from *first full exercise
  of every component* into a *repeat*. This is the single highest-value
  remaining de-risk and the direct answer to the one-shot constraint.

One-shot-specific failure ranking (post-gates): mid-download
HTTP/BPP stall with partial install > credential-reissuance policy
unknown (owner-side: confirm silent.link reissue terms BEFORE the
window — if reissuable, the pressure drops materially) > transport
surprise on the chosen path (covered by Gate 1) > composition wedge
(recovery documented; card untouched) > enable-phase quirks (ladder +
rollback; not one-shot) > post-enable registration (rollback to prior
profile documented).

### Ownership assignment (2026-10-25, owner-directed coordination)

| Role | Lane | Status |
|---|---|---|
| Artifacts + checklist (Gate-2 build: pin + #452 @ `18c1b7d` on `route1-tls452` + minimal upstream-reportable segfault fix; T-A/T-B checklist phases) | w19:pK | ACCEPTED within timebox; mipsel SDK build in flight; #459-Criterion configure quirk handled in wrapper; mock dry-run evidence to be attached atcsimUC-style |
| Window execution (Gate-1 read-only → T-A → T-B, lane lock, per-phase owner GO) | w19:pH | Accepted **by action**: atcsimUC deployed to a disjoint tree (`/tmp/ucroot` + `LD_LIBRARY_PATH` — resolves the deployment-order requirement D without overlay ordering), UC fingerprints verified **on-router**, private expectations set (`EXPECTED_QCCID`/`EXPECTED_SLOT=2`), read-only window running under lane lock `window-arch-r1r2` |
| Gate review between phases | w19:pJ | standing |

Notes: (1) Gate-1 execution is covered by the owner's standing read-only
milestone directive; **T-A/T-B remain write-gated**. (2) pH flagged
2.9 MB overlay free — sufficient for the read-only rungs; package
installs for later phases need cleanup first. (3) pK's commit semantic:
Gate-2 build exists before T-A. (4) Owner-side item in parallel:
silent.link reissue policy.

### Gate-1 live results — VERIFIED by this review (2026-10-25, window
`window-arch-r1r2`, lane lock taken/released; two runs, evidence
attributed per run)

Verified from the host-side captures (`window-run.log` 10:12 run,
`window-run2.log` 10:18 run) — the milestone reports are real:

- **R1a stock (run 2)**: `exit=255`, `AT_DEBUG_RX: ERROR` on the
  lowercase ISD-R SELECT — the pySim fingerprint **live on our modem**,
  third independent confirmation (mock, battery case-control, now live).
- **R1b atcsimUC (run 2)**: `exit=0`, **complete EUICCInfo2 JSON over
  AT+CSIM with the operational profile ENABLED** — a full LPA session
  (open → ISD-R select → ES10 reads) on the production card, on-router.
  **"Enabled profile blocks CSIM LPA sessions" is CONTRADICTED.**
- **R2 battery (run 1; run 2 SKIPped — later diagnosed as a cosmetic
  message bug, rung deselected by the deployed revision; fixed
  `dd6cddd`; evidence attribution corrected from the merged report and
  now folded lane-side as correction 5 `b535fa0`)**: verified
  **host-side from the preserved router artifacts** (`gate1-20261025/
  router-artifacts.tar.gz`, sha256 `780af30d…a221`, re-hashed and
  extracted by this review):
  - `AT+CSIM=10,"02C0000021"` (CLA=02, **INS C0**) →
    `+CSIM: 70,"6F1F8410<AID>A504…9000"` — **the real ISD-R FCI**,
    on channel 2 and channel 3, deterministic across gap variants.
    "GET RESPONSE never works" is **retired with byte-level evidence**.
  - Inline-FCI SELECT (P2=00, Le=00) on a properly-addressed channel →
    `6121` — the historical `6A82` is confirmed as the CLA-provenance
    artifact.
  - `leaked closes: 4 (channels: 2 3)` — close-refusal tally verified.
- Card facts (private logs only): EID matches census; free NVM
  409,335 B; `rspCapability: additionalProfile, testProfileSupport`
  (T-A/T-B card-supported); CI PKID for verification/signing = RSP2
  Root CI1 SKID — the TLS-chain root matches the card's key-verification
  list end-to-end.

**Architectural consequence**: the provisioning transport is decided —
**AT+CSIM via the atcsimUC-class tree**. No composition switch, no WAN
migration, no overlay-space package installs are required for T-A, T-B,
or the silent.link one-shot. Route-1's composition phase is demoted to
the long-term reader workstream. **Gate-2 artifact spec amended
(urgently, to pK): atcsimUC (uppercase + #446) + #452 + segfault fix** —
a pin+#452-only build would die at the first SELECT on the now-chosen
path.

Standing-question verdicts updated: Q1 void (unchanged), Q2 answered —
**contradicted**, AT+CSIM provisions enabled-profile eUICCs (client
case-bug was the blocker); Q3 answered — lowercase rejection, confirmed
live; Q4/Q5 mooted for provisioning (QMI remains long-term reader
option); Q9 superseded — safest path today is the proven AT transport
+ hardened Gate-2 build.

Additional record items (2026-10-25, post-Gate-1): run-1's exit-127s
cause-named (no `timeout` applet on stock OpenWrt → the session
script's `run_to()` watchdog fix, `7a426a9`); pH's execution ownership
ACCEPT is formal and standing; T-A activation-code prerequisite
**resolved — the Speedtest credential is public**
(`LPA:1$rsp.truphone.com$QRF-SPEEDTEST`, osmocom Known-Test-Profiles
page, verified this session): stage verbatim in the private runbook
(single-quote discipline); no owner-side acquisition path exists for
T-A — only the silent.link one-shot requires private credential
staging.

### Gate-2 build (w19:pK, `route1-gate2`) — spec applied in time; reviewer-verified elements (2026-10-25)

The urgent spec amendment (atcsimUC-base) landed while the mipsel SDK
was still fetching. Commit stack: pin → uppercase (`1982d5e`) → #446
(`2c65cf6`) → #452 (`da047ad`) → segfix-zero-init (`d68be3b`). Verified
first-hand by this review: the x86 pre-verification build carries the
uppercase table in `libeuicc.so` and the `LPAC_HTTP_SSL_VERIFY` marker
in `driver_http_curl.so`; the gdb capture (`segfix/gdb3.log`) confirms
the segfault root cause — `SIGSEGV in __libc_free(mem=0x3)` inside
`es10c_ex_euiccinfo2_free` — i.e. teardown walking **uninitialized
`_cleanup_` locals** (11 across 8 applet files) when an ES10 call fails
early; the fix is mechanical zero-init with no success-path change.
This completes the upstream-report content (the two owner-gated
artifacts now have full root-cause + reproducer material). mipsel build
in flight (daemon-owned after a reaped-process-group hazard killed two
attempts); fingerprint + sha256 handoff to pH on landing. Preflight doc
amended (`ccad88b`): AT-pivot (composition switch demoted to annex G),
T-A/T-B phases, card facts, TLS-validation datum, reissuance note.

### T-A TLS prep (2026-10-25, pH) — bench datum verified-class; router-gate gap flagged

Bench precheck of `rsp.truphone.com` with the GSMA-augmented bundle:
chain `CN=1GLOBAL SM-DP+ ← GSM Association RSP2 Root CI1`, TLSv1.2
ECDHE-ECDSA-AES256-GCM-SHA384, verification passing; plain store fails
(error 21, root absent — same shape as the Idemia endpoint). Same CI1
root the card binds for verification (Gate-1 PKID): the write-window
crypto story is coherent end-to-end. Speedtest LPA string staged in the
private runbook.

**Gap flagged (reviewer)**: the router has no `curl` (busybox wget
only), so the §A3 on-router TLS gate as written cannot run — busybox
wget exposes no CA-bundle control and typically rides libustream-
mbedtls, i.e. possibly the same failure class under remedy.
Resolution: **acceptable for T-A** — the test credential is reusable,
the download itself exercises the Gate-2 verified curl leg, and any TLS
failure during T-A is free/diagnosable. **Not acceptable unresolved for
the silent.link one-shot window** — before that gate, decide one of:
install curl on the router (overlay-space permitting, 2.9 MB
constraint), use the Gate-2 lpac HTTP leg as the prober, or explicitly
accept host-side-only precheck with the T-A result as the router-side
evidence. T-A blockers unchanged: Gate-2 artifact + owner GO.

### Owner GO logged + T-B review conditions (2026-10-25)

Owner GO, verbatim scope: "coordinate with the other agents + flash a
test profile from Speedtest when safely able." Reviewer concurs with
the execution lane's reading: the GO covers T-A and T-B with
review-between-phases intact; no separate owner gate — **given five
binding T-B conditions** (the operationalization of "safely able"):

1. T-A review passes first, including the notification-process write
   completing cleanly, post-install free-NVM, and complete captures.
2. VLAN200 failover **confirmed live** immediately before enable
   (tollgate continuity under nadanada displacement) — verified, not
   assumed.
3. Rollback pre-staged: the exact re-enable-nadanada-by-ICCID command
   present in the window runbook before enable executes.
4. Time-boxed displacement: enable → speed test → re-enable in one
   continuous supervised session; the card is never left displaced
   unattended.
5. Post-T-B verification complete (nadanada re-enabled, registration +
   QCCID, tollgate restored) before any optional profile deletion.

Default-stop: any T-A anomaly holds T-B pending owner consultation.

### T-B incident (2026-10-25, executing-lane report; reviewer recorded)

T-B executed through: VLAN200 live-check OK → notifications processed
(nadanada-era acks cleared; one install ack remained) → **enable-by-ICCID
SUCCESS** → the post-enable `AT+CFUN=1,1` **wedged the modem off the USB
bus**. Measurement never ran; rollback impossible (no transport). Card
state: persistent and safe — test profile enabled, operational profile
intact-but-disabled. Lane lock released; full private incident record.

Recovery ladder climbed in documented order, all ineffective: host soft
reboot; RESET_N (uci gpio515, 3 s/30 s); USB-power trick n/a (device
electrically absent); sysfs gpio export EBUSY (hog — structural);
driver-unbind dead-end (verified); /dev/mem compiled out; PCI path n/a.
Final rung — a purpose-built `gpiopoke` kernel module (dump-first:
register map confirmed live at the MT7621 GPIO window — DIR/DATA/SET/CLR
at +0x00/+0x20/+0x30/+0x40, bit18 = modem power rail, bit4 = pwrkey;
then `cycle` mode: register-verified 30 s rail cut, DATA 1→0→1; then
`pwrkey` mode: 12 s force-off + 2.5 s boot press per Quectel semantics).
**Zero USB attach events in any mode, no EDL** — the modem did not
respond to any electrical stimulus. Module self-unloaded, hogs restored,
`.ko` volatile, source bench-side — the reviewer endorses landing the
source under `tools/` as a documented recovery instrument for this
hardware family (it executed flawlessly at register level).

Correlation for the record: the RM551 field report (lpac + 9eSIM on
Quectel — modem dead after reboot "until some attempts") plus this
incident define a **post-profile-operation reset instability class** on
Quectel+9eSIM combinations; the settle-probe discipline in the
checklist was designed for it and should stay mandatory.

Owner decision pending on four options; reviewer assessment delivered
(physical power cut first — strictly stronger than the kmod rail cut
because a full cut also drops VBUS, which the load-switch rail does
not).

Execution-staging confirmation (pH, same day): the five
conditions are staged verbatim in the private runbook (`T-B BINDING
CONDITIONS` section) including the pre-staged re-enable one-liner —
binding condition 3 closed before the Gate-2 artifact landed. Awaiting
artifact (mipsel build in flight, watcher set); reviewer turnaround
same-hour on receipt.
### Gate-2 artifact review: PASS (2026-10-25, this review — all first-hand)

`lpac-mt7621-gate2-mipsel_24kc.tar.gz` sha256 `b587d46f…aa9b44`
(exact match, re-hashed on disk); git stack `route1-gate2` verified:
`d68be3b` (zero-init segfix) ← `da047ad` (#452) ← `2c65cf6` (#446) ←
`1982d5e` (uppercase) ← `a43e46a` (pin) — the amended spec to the
letter. Fingerprints in the correct objects (uppercase table in
`libeuicc.so`; `LPAC_HTTP_SSL_VERIFY` in `driver_http_curl.so`;
`at_expect_with_deadline` in `driver_apdu_at_csim.so`). NEEDED-closure
clean; libcurl is openssl-bound with `libssl`/`libcrypto`/`libatomic`
**bundled — self-contained TLS remedy**. Deployment note recorded: the
bundled `usr/lib/libcurl.so.4` lands at the same path as the router's
system (mbedtls) libcurl — extraction deliberately replaces it,
superseding the gnutls symlink-repoint remedy; post-window restore =
reinstall feed libcurl or adopt (owner choice). Build-process note: a
template bug briefly produced a pin-only artifact (`5b03fafe`),
caught by the in-manifest stack line before any consumption or router
contact; branch-guard added; superseded shas documented. **T-A
proceeds on pK's co-sign per the two-party gate.**

Co-sign issued (pK, same day): sha re-verified exact; F1–F4 first-hand
(F2 strengthened with `timerfd_create` import as the structural #446
marker; DT_RPATH + loader-source construction match for plugin
discovery); their qemu behavioral test left incomplete (harness
limitation, noted honestly in the sign) with basis = structural +
Gate-1 R1b live-convention proof + this review's PASS. Checklist of
record updated (`faa2b91`) incl. the libcurl-replacement consequence
and inner shas. **Two-party gate CLOSED — T-A cleared to execute under
pH's lane lock with the owner GO standing.**

### T-A EXECUTED: SUCCESS — review PASS, T-B released (2026-10-25
18:37–18:39Z, lock `ta-speedtest`, released; artifacts `3eb690b7…`,
re-hashed and verified by this review)

Verified first-hand from the preserved captures: complete SGP.22 chain
over AT+CSIM (challenge → mutual auth → BPP install, `seqNumber:2`,
24 s); operational profile `enabled` and untouched throughout; new
Speedtest/BetterRoaming profile `disabled` as designed; ES10x auto-C0
chaining working in production traffic (516-byte profile-info blobs);
TLS via the bundled openssl libcurl; 1032-line wire capture. Process
note accepted: execution proceeded on the material two-party basis
(this review's PASS + pK's visible four-point PASS of the identical
sha) with pK's formal co-sign landing afterward, no discrepancy.

**Correction-6 class finding (this review, from the T-A wire): the
channel-close "defect" was a probe artifact.** lpac's 5-byte close
(`0070800200`, P3=00) returns `9000` — twice in these captures. Every
prior close-refusal used the 4-byte form, which this modem treats as a
malformed APDU (consistent with its documented bad-Lc drop). The
three-defect model collapses to **one real defect — the client-side
lowercase emission, fixed in-tree**. No channel leak exists in
lpac-driven sessions; the exhaustion mechanism loses its driver.

T-B flow amendments with the release: open with
`lpac notification process -a` (clears 3 pending — including the two
queued since the original v4.0-era provisioning — hygiene plus the
designed rehearsal write) before enable; the nickname proposal is
dropped (non-empty queue; no persistent cosmetic card writes); process
notifications once more after rollback. Binding conditions 2–5 stand
as staged.

### Owner directive (2026-10-25): test profiles FIRST





The rehearsal download is promoted from recommendation to plan of
record: provision a **free, reusable test profile before the silent.link
one-shot**. Selection: the **Speedtest profile**
(`rsp.truphone.com`, matching-id `QRF-SPEEDTEST`, per the osmocom
Known-Test-Profiles page) — unlike BetterRoaming profiles (freely
downloadable but subscription-gated for registration), it is designed
to register and carry data for actual speed tests, matching the owner's
intent. Bonus validation: the canonical #24963 mbedtls failure was
reported against this very endpoint — the test exercises the identical
TLS-remedy path as the real window.

Phasing (mandatory — the enable displaces the operational profile):

- **Phase T-A — machinery rehearsal (download only)**: free-NVM gate →
  `lpac profile download` of the Speedtest profile → verify
  present-but-disabled → notification process → machinery proven.
  Card-state impact: one disabled profile added; deletable. nadanada
  untouched (enabled throughout). Failure here is *free* — diagnose and
  retry at will; that is the point of running it first.
- **Phase T-B — enable + speed test + rollback (separately gated)**:
  enabling the test profile **displaces nadanada** (non-MEP) — tollgate
  rides VLAN200 for the duration; run the speed test; then re-enable
  nadanada (the documented enable ladder in reverse) and verify
  registration + QCCID before restoring the tollgate path. Optional
  cleanup: `lpac profile delete` of the test profile afterward.

Sequencing unchanged otherwise: read-only window (Gate 1) still runs
first — its outcome picks the transport the test download rides; the
hardened build (Gate 2: #452 + segfault fix) is required before T-A
(the truphone endpoint is itself in the mbedtls-failure class).
Remaining owner-side item: silent.link reissue policy (parallel, before
the one-shot).

---

## E. Recommended phased roadmap

1. **Evidence corrections and low-risk research (now)** — done in part:
   `a46625c` landed; this review distributed; PR #30 comment correction
   note queued for the PR's next touch. Remaining: D2's query-only
   slot-mapping capture (read-only, router-side, no card access beyond
   what the gate does); README-vs-round-4 B0 divergence queued to the
   next hardware lane round.
2. **Sacrificial/approved validation** — the MBIM path's first live
   exercise is necessarily the production unit (no second RG502Q);
   therefore the read-only gate D3 *is* the sacrificial round, strictly
   ordered before any write. If an external OmniKey + disposable eUICC
   can be borrowed for one lpac-mbim smoke on the bench host, it
   cheaply retires the backend-on-eUICC unknown first — recommended if
   available, not blocking.
3. **Near-term provisioning path selection** — reaffirm Option C
   (runbook v3) at the owner gate once D2/D4–D6 close; Option A remains
   the fallback *only if* C's gate fails at step 2 (then: corrected C0
   battery becomes pre-window-blocking instead of post-window).
4. **Pre-provisioning authorization gate** — owner GO on the runbook
   with gates D1–D8 recorded green; explicit acknowledgment that the
   first write is notification-processing (reversible class) and the
   only profile-state-changing step is the enable ladder.
5. **Controlled download and activation** — runbook steps 4–6 unchanged
   (one-shot download, enable ladder, contingency-disable only on
   ladder failure, verify + rollback to prior profile on registration
   failure).
6. **Post-installation verification and recovery readiness** — runbook
   steps 6–7 (CFUN recycle, ICCID/CPIN/CEREG verification, composition
   revert or adoption decision, D1 reconciliation in the operator's
   database, audit).
7. **Longer-term reader architecture (post-window)** — in order:
   (a) corrected CSIM probe battery (C0 GET RESPONSE on allocated
   channels, case-4 SELECT P2=00+Le, C0-on-basic control, paced
   repeats, lpac with wire-level APDU debug on a fresh CFUN-recycled
   state) — decides whether Option A revives; (b) PR #30 disposition
   per §F; (c) if a general PC/SC surface is still wanted, prototype
   Option F (vpcd or pcsc-relay-style agent over the MBIM/QMI backend)
   reusing the ownership/timeout disciplines already hardened on the
   ssh path; (d) revisit wwand as production tooling (mipsel_24kc feed
   packages) for ongoing eSIM management.

---

## F. Decision log

| Decision | Verdict | Rationale | What would reopen it |
|---|---|---|---|
| GET RESPONSE evidence | **Void** (accepted by all lanes, `a46625c`) | constructor used INS 2C; no C0 in any surviving artifact; wire provenance absent | any archived log showing a genuine `C0` exchange |
| "Enabled profile blocks CSIM LPA" | Downgraded to hypothesis | central pillar voided; simpler unexcluded explanations (channel exhaustion; modem-owned T=0 procedures) | corrected battery results either way |
| Near-term route | **Keep MBIM (runbook v3), no re-pivot** | legs independent of CSIM theory (vendor channel, wwand exact-modem read-class proof, buildability, artifacts); re-pivot = schedule risk for zero gain | D3 gate failure (MBIM blocked on target) → Option A escalation with corrected battery as blocker |
| lpac MBIM slot handling | **SKIP_SLOT_MAPPING first; UIM_SLOT=2 mapping only if skip demonstrably broken** | mapping-write risk on an Unknown-reported modem; active card already correct | query-only capture showing mapping reliably matches with no write, or skip-mode malfunctioning |
| QMI backend | Deferred (not near-term) | feed libqmi compiles out the UIM APDU surface; self-build = new surface for zero gain | a feed/SDK update shipping UIM APIs, or a long-term decision to own a libqmi build |
| PR #30 | **Deferred + re-scoped; not a dependency of the window** | five verified defects incl. doctrine violation; transport fragility on enabled-profile cards unresolved | fixes for §A.6 items 1–5 + corrected-battery outcome on the AT transport |
| Disable-before-download | **Rejected as a default step; contingency only** | SGP.22 download is ISD-R-scoped/additive; vendor tooling operates on enabled-profile cards; preservation asymmetry | enable-ladder failure indicating profile-state interference (runbook's own contingency path) |
| External reader | Last-resort fallback (staged, triple-gated) | class-proven but off-product workflow; product constraint prefers on-device | D3 gate failure |

### Owner-constraint addendum (2026-10-25 evening broadcast — final)

- **External CCID reader eliminated by owner constraint** (not a
  technical review outcome; the #459 review amendments stand as record,
  the concurrence draft was never posted — moot). Option D is struck
  from the matrix.
- **Card stays seated in the NR7101.** The v4.0 shape (host pcscd +
  NR7101 as reader from a separate computer) is out of scope.
- **Next milestone: read-only 9eSIM operations from tooling compiled +
  installed on the NR7101.** AT commands and pcscd/lpac-on-router are
  both in scope. No disagreement-ledger entry — a constraint, not a
  claim.
- **Matrix effect**: Option A splits into *AT-on-router* (lpac at_csim;
  the mipsel artifact is already deployed on the router) and
  *A-router-PCSC* (on-router pcscd + this driver over a loopback bridge
  — note the driver's direct-serial transport is unimplemented; the
  loopback socat bridge shape is required). Option C is unchanged and
  its read-only gates (D2/D3) are promoted from gate to milestone
  content. Option D struck.
- **Reviewer ordering input** (for the architecture lane's pending
  ordering verdict — evidence-ranked, all read-only, all on-router):
  **R1** lpac at_csim `chip info` immediately after a CFUN recycle,
  with `AT_DEBUG` wire capture (source-verified env in the pinned
  at_csim backend; artifact already on-router) — decides §A.3
  (exhaustion hypothesis) at zero deploy cost. **R2** the corrected C0
  battery (`ade4c77`, length-audited, class-A read-only, under
  modem-lane lock) — decides §A.4 mechanisms. **R3** the MBIM read-only
  gate (single composition switch, `SKIP_SLOT_MAPPING` protocol D2,
  chip-info soak D3) — the only rung that also unlocks later
  provisioning. **R4** on-router pcscd + driver + loopback bridge —
  pursue only if a PC/SC surface on the router is itself a goal; it is
  not needed for lpac-direct provisioning. Every R1/R2 outcome
  cheapens or redirects R3; R3 remains the strategic unlock.
- Reopeners: none for the constraint itself; existing rows unchanged.

### Ordering verdict reconciliation (2026-10-25, architecture lane + reviewer)

The architecture lane adopted R1/R2/R4 unchanged and split R3:
**R3a** = feed lpac (2.3.0, official OpenWrt packages, uqmi default
backend) over the factory QMI composition (`usbnet=0`), uqmi already
on the unit — the only exact-modem wire-proven transport class;
**R3b** = the MBIM gate as fallback (dually-verified artifact,
`SKIP_SLOT_MAPPING`). Reviewer verdict: **CONCUR with the split —
with three amendments** (recorded, no dispute):

1. **A1 — slot guidance corrected (must-fix).** The lane's caveat
   "`LPAC_APDU_QMI_UIM_SLOT` defaults 1 → MUST set 2" is inverted for
   this modem. Three independent sources agree: (i) wwand's exact-modem
   measurement (2026-09-26): both physical slots map to **logical slot
   1**; `SEND_APDU` on logical slot 2 is refused `NOT_SUPPORTED (94)`;
   a channel "opened on slot 2" lands on the active card; (ii) lpac's
   uqmi backend documents `UIM_SLOT` default `1`, 1-based; (iii) uqmi's
   own `--uim-slot [1-2]` is per-command addressing, not a mapping
   write. The eUICC is the **active** card (slot state verified in the
   probe logs) → logical slot 1 → **leave the default**. Setting 2
   aims at the refused logical slot. Note the hazard classes also
   differ from MBIM: uqmi cannot write slot mappings, so a wrong value
   fails loudly rather than switching cards.
2. **A2 — "factory end-state, no second switch" needs one honest
   caveat.** The unit's *current* baseline is ECM (`usbnet=1`) and the
   tollgate WAN rides it. Ending at `usbnet=0` requires a one-time WAN
   migration (netifd `proto qmi` + uqmi — both present on the unit) or
   a switch back to ECM afterward. During the switch the ECM iface
   disappears — the D5 failover/VLAN200 discipline applies to R3a
   exactly as it did to the MBIM plan.
3. **A3 — zero-cost client pre-check before any switch.** Verify the
   installed uqmi's UIM APDU surface *before* touching composition
   (help text needs no modem). Verified upstream this session: OpenWrt
   uqmi `commands-uim.h` carries `uim-channel-open`, `uim-channel-close`,
   `uim-apdu-send`, `--uim-slot [1-2]`; lpac upstream documents the
   uqmi backend for OpenWrt (`docs/backends/qmi.md`).

Supporting external record (this session): lpac packaged in official
`openwrt/packages` (`utils/lpac`); feed TLS caveat confirmed
(openwrt/packages#24963 — libcurl+mbedtls SM-DP+ incompatibility class;
read-only ops unaffected, write-time wants the libcurl-openssl
variant); MBIM-devices-vs-uqmi-backend mismatch reported
(openwrt/packages#26232) — reinforces QMI-composition-for-uqmi pairing.
Lane cross-cutting flags endorsed: mbedtls/TLS (above), the
400 ms-vs-8 s CSIM pacing discrepancy to reconcile before interpreting
R1 (v4.0 floor vs conwrt-bench #22 claim — different eras; R1's
timestamped wire capture resolves it), and AT-port shuffle
auto-detection (standing driver doctrine).

### R-½ QUIMSLOT rehearsal cycle — reviewer sign-off (2026-10-25)

**APPROVED — safe class, with mandatory guards.** Basis: `AT+QUIMSLOT`
is documented, NVM-persisted, reset-applied, and is NOT the
`QESIM lpa_enable` do-not-toggle wedge class; the original probe script
v2 already implemented a sanctioned switch cycle with abort-safe
restore (it fired no switch only because the slot already read 2).
Switching de-powers the inactive slot's card — the production eUICC is
card-content-untouched and *structurally unreachable* for the whole
rehearsal, which is a safety feature, not a risk. Restore is one
command + reset.

Mandatory guards:

1. **ICCID-verify before any op**: after the switch, `QCCID` must match
   the slot-1 test USIM. ABORT if the eUICC's ICCID appears — the
   rehearsal must never run against the production card.
2. **Restore discipline**: trap-based restore guard + explicit restore
   + post-restore verification (`QUIMSLOT?` = eUICC slot, `QCCID` =
   eUICC ICCID, `CPIN: READY`). NVM persistence means restore is
   mandatory even after an unexpected reboot.
3. Tollgate failover (VLAN200) confirmed before start — the two modem
   resets bounce the WAN (~2 min class, twice).
4. modem-lane lock + heartbeats throughout.
5. Class-A read-only op set only (open / SELECT / read / close — the
   `validate-sacrificial.sh` shape); no QESIM/QSIMCFG toggles while on
   slot 1; pre-declared command list.
6. Budget: two resets total; abort on double re-enumeration failure.

Extra value: the cycle rehearses the USB re-enumeration handling that
R3a's composition reboot needs, one rung earlier. Reviewer concurs
with the lane's framing: the rehearsal is comfort, not safety — if
declined, the ladder runs R1→R4 directly with every rung read-only.

### Fork-ecosystem findings (owner question, 2026-10-25)

- **`fildunsky/lpac-build`** (active 2026-09; targets incl. our exact
  `ramips/mt7621 mipsel_24kc`, OpenWrt 25.12 apk): packaging for a
  patched lpac 2.3.x ("FM350 eSIM via native AT driver"). Two material
  findings:
  1. **Curated open AT-driver PRs absent from our pin**: estkme lpac
     #446 (bounded poll deadlines in `at_expect`), #447 (tolerate
     *silent* AT capability probes), #448 (bounded CGLA/CCHC deadlines
     for profile download) are all OPEN upstream — verified this
     session — so `a43e46ae` (= main) lacks them. #447 matches a
     documented RG502Q behavior (AT test commands answer bare OK, or
     stay silent, per the driver README) and is a plausible direct,
     client-side cause of the `euicc_init` ×3 failures — a competitor
     to the channel-exhaustion hypothesis. **R1 amendment**: run the
     discrimination with two builds — stock `a43e46ae` and
     main+#446/#447 — a client-side fix flipping init to success would
     re-rank the whole AT path without touching the card.
  2. **libqmi tension**: their README states native QMI (+MBIM)
     cross-builds cleanly on 25.12 with feed glib2 deps — in tension
     with our verified compiled-out UIM headers (SDK staging, libqmi
     1.36.0). Possible resolution: the lane's staged libqmi may be a
     reduced configuration vs the feed binary package. Cheap re-check
     for the lane: inspect the *feed binary* package's headers (not
     the SDK-staged copy). If the feed binary carries UIM APIs, the
     native `qmi` backend revives as a fallback class (uqmi stays
     first: zero artifacts). Version floor noted: lpac qmi driver
     needs qmi-glib ≥ 1.35.5 (1.36.0 satisfies).
- **`pipe/lpac-quectel`** (2024-07, stale): stock README; fork value is
  historical only — its Quectel AT workarounds predate the at_csim
  backend era. Not actionable.
- **Same-family corroboration for R3a**: Khadas-forum RM500Q-GL eSIM
  on Linux — works in QMI usbnet composition with a QMI-aware option
  driver ("only works in usbnet mode"); modem-firmware sensitivity
  noted by the reporter. Consistent with our QMI-composition-first
  ordering; another data point against assuming firmware-generation
  uniformity.

### Lowercase-hex smoking gun (2026-10-25, w19:pK handover — independently verified by this review)

The deep-dive lane's handover (`docs/FORK-RESEARCH-LPAC-QUECTEL-LPAC-BUILD.md`,
commit `b31bc36`) supersedes this review's #447 hypothesis above. **The
reviewer's #447 mechanism was wrong** (those PRs patch `at_etsi.c` — the
CGLA backend — not `at_etsi_csim.c`, and at_csim's capability probe
already passes on bare OK). The replacement hypothesis, verified
first-hand by this review against the pinned checkout:

- lpac `euicc/hexutil.c` emits **lowercase** hex (`digits[] =
  "0123456789abcdef"`), so every at_csim APDU crosses the wire
  lowercase at `a43e46ae`, at upstream main today, and in fildunsky p9.
- This project's driver emits **uppercase** (`src/at.rs`, ABCDEF table)
  and carried the complete v4.0 provisioning on this exact modem —
  lpac's lowercase never reached the modem in that run (pcsc backend).
- Field evidence for Qualcomm-class lowercase rejection: pySim gerrit
  8e15c18 (Sierra EM7565: lowercase → bare ERROR, uppercase → works);
  lpac #428 (Foxconn T99W696: at_csim `euicc_init` failure — **the
  bench's exact error signature** — with a one-line digits fix
  attached, still unmerged upstream).

Consequences:

- The bench `euicc_init` ×3 failures now have a complete **client-side**
  explanation that requires neither channel exhaustion nor any
  enabled-profile mechanism. Channel exhaustion is demoted to a
  secondary, unexcluded hypothesis for those runs.
- **R1 amended (pre-registered)**: discrimination builds = stock
  `a43e46ae` vs pin + uppercase hexutil (+ #446 to bound silent hangs —
  the pinned `at_expect` uses `poll(-1)`). Prediction: with AT_DEBUG,
  the stock build's capture shows lowercase `AT+CSIM=…` answered
  `ERROR` (pySim fast-fail shape — matches bench) or silence (hang
  shape); the uppercase build's same commands get `+CSIM:` responses
  (probe-log precedent). Either outcome is decisive; the capture makes
  it evidence either way.
- The corrected findings doc's lpac row needs a third annotation at its
  next touch: leading hypothesis = lowercase-hex AT rejection
  (client-side), exhaustion secondary.
- The §A.4 anomalies (`61 21` decoupling, `6A82`) are NOT explained by
  case (all probes were uppercase) — R2 remains necessary for those.

Additional handover verdicts, accepted and recorded: **libqmi tension
resolved** (UIM logical-channel messages exist only in the FULL
collection; defconfig picks BASIC — the compiled-out mechanism;
fildunsky `d27874e` forces `CONFIG_LIBQMI_COLLECTION_FULL=y`; version
floor fine — config, not version); fildunsky ships a ready-made
`lpac-25.12.5-ramips-mt7621.apk` (v2.3.0-r4) as a third discrimination
binary; the official 25.12 feed lpac has a **broken stdio backend**
(their LuCI README); lpac #450 (open) is the in-tree opt-in SIM
power-cycle after profile toggle; the lpac manpage documents
ECP-removable-eUICC **enable quirks** (try AID → ICCID → refreshFlag —
matches our enable ladder) — a write-window datum for our ECP3; RM551
field report (lpac + 9eSIM cards on Quectel works, but AT/QMI APDU is
dead after reboot until "some attempts") sets the **post-switch
settle-probe expectation for R3a**.

### Offline R1 rehearsal — PREDICTION CONFIRMED (2026-10-25, this review)

Full offline discrimination executed on the bench host (no router, no
card): stock `a43e46ae` vs one-line uppercase-hexutil build, both x86,
against `mock_modem.py --case-sensitive` (new fingerprint mode,
committed) over a pty bridge. Wire captures on file
(`/tmp/opencode/r1-{stock,upper}-*` + mock JSONL):

- **Stock**: connect probes pass (`AT`, `AT+CSIM=?` → OK — refuting the
  #447 mechanism in flight), MANAGE CHANNEL open `0070000001` →
  `019000`, then ISD-R SELECT **lowercase** (`01a4040010a000…`) →
  **bare `ERROR`** → `{"code":-1,"message":"euicc_init"}` — the bench
  signature, reproduced with the mechanism visible on the wire.
- **Uppercase build**: same open → SELECT **uppercase** →
  `+CSIM: 16,"6F048402A0009000"` (**FCI answered, 9000**) → proceeds to
  ES10 getEID STORE DATA (`82E2910006BF3E015A`-class) → mock (no E2
  implementation) answers `6D00` → lpac prints the `es10c_get_eid`
  error JSON **then** core-dumps (w19:pK frame-by-frame: the crash is
  post-error cleanup, reachable on *any* card-error SW ending an ES10
  exchange — **R1 interpretation caveat: a live die-after-error-JSON is
  this bug, not a transport finding**).
- Response parsing is case-folding upstream (`input[i] | ' '`), so the
  uppercase patch cannot break RX.

Confidence change: the lowercase-rejection hypothesis moves from
**Hypothesis** to **Strongly supported** (client-side chain fully
demonstrated against a fingerprint-accurate mock; the remaining live
unknown is only whether the real RG502Q parser behaves like the
fingerprint — which is precisely what R1 on-router measures). The v4.0
success (driver pcsc path, uppercase) and the bench ×3 failures
(lowercase at the ISD-R SELECT, each failed run leaking its opened
channel) are now reconciled by one mechanism with exhaustion as a
secondary compounding effect.

**Defect-independence clarification** (reviewer, sharpening the
"symptoms" framing): three *independent* defects are now in evidence,
none requiring the enabled-profile theory — (1) the client-side
lowercase rejection (lpac TX; primary cause of the bench ×3), (2) the
modem's channel-**close** refusal (observed with uppercase probes too —
a real firmware defect in its own right, NOT a symptom of the case
bug), and (3) channel-exhaustion on open (`6A81`), which is the
*downstream symptom* of (2) plus failed runs leaking channels. In the
old bench narrative, (3) was read as evidence of session incoherence;
it is now a symptom of (2). The `61 21`/`6A82` SELECT anomalies remain
the only unexplained observations (R2's scope).

**Upstream reporting decision** (reviewer's call on the bundling
question): do **not** bundle. File two separate, owner-gated artifacts:
(a) a comment on the existing lpac #428 adding RG502Q-EA as a second
Qualcomm-class data point — wire captures from the rehearsal plus the
maintainer's own unimplemented `LPAC_APDU_AT_HEX=UPPER` suggestion; and
(b) a new issue for the post-error-cleanup segfault (own reproducer and
transcript), cross-referencing #428 only. Separate threads for separate
bugs keeps triage clean. Sequencing: file **after R1's live wire
capture** — the offline prediction is strong, but the upstream claim
should carry the live confirmation, and the report must stay
species-level (mock artifacts and public AIDs only; no bench context,
no identifiers).

R1 on-router is now a *confirmation* run, not an exploration run.

### Readiness verification (2026-10-25, reviewer — all first-hand)

Every load-bearing claim from the lanes' final deliveries was verified
directly, not taken on report:

- **Correction 4** on `findings/csim-session-state`: commits `8593e4a`
  (+ fixup `aae7018`) pushed; matrix row carries the third annotation;
  §8.6 "AT transports closed" marked no-longer-established.
- **R1 discrimination artifact** `lpac-mt7621-atcsimUc-a43e46ae.tar.gz`
  sha256 `fed4c6c1…4b6` — re-hashed on disk; extracted and
  fingerprinted: `0123456789ABCDEF` present exactly once in
  `libeuicc.so`, `at_expect_with_deadline` present in
  `driver_apdu_at_csim.so` (PR#446 applied), absent from the wrong
  objects.
- **R3a artifact** `lpac-mt7621-uqmi-a43e46ae.tar.gz` sha256
  `e5e04429…5d8` — re-hashed on disk.
- **R2 battery**: `at_csim_c0_battery.sh` + `mock_c0_responder.py`
  in-repo (commit `efddc55`) with the hex-case control cell and the
  `*9000` verdict-glob fix; 3/3 rehearsal scenarios green (including
  the harness's own PTY-echo bug found and fixed by that rehearsal).
- **Three-way defect split** landed in the research doc (`b154855`) as
  the reviewer required (close-refusal kept at independent-defect
  status).

**Position**: every offline-validatable element of the ladder is now
validated and staged. The remaining unknowns — the live modem's hex-case
behavior (R1), the `61 21`/`6A82` mechanisms (R2), uqmi-client on AACR
(R3a), and MBIM-on-RG502Q (R3b) — are measurable only on the router,
under lane discipline and owner gates. Upstream reports (#428 comment +
segfault issue) drafted, owner-gated, sequenced after R1's live capture.

### Session-script review — `onrouter-readonly-session.sh` (a967071, 2026-10-25)

Verdict: **AMEND** — structure, preflight set, read-only discipline,
interpretation reminders, and timeout bounds endorsed; five items
required before the runbook records it:

- **(A) Blocking — wrong env var**: `LPAC_QMI_DEV` is not read by the
  at_csim backend; both invocations need `LPAC_APDU_AT_DEVICE`
  (source-verified `at_cmd.h`; the offline rehearsal used it).
- **(B) Blocking — blind AT battery**: the loop writes to the tty but
  never reads it, so the state battery captures nothing; needs the
  background-reader pattern *plus* explicit abort checks (QUIMSLOT =
  eUICC slot, QCCID = eUICC ICCID, else stop). A battery that cannot
  fail verifies nothing.
- **(C) Structural — no CFUN recycle**: pre-existing leaks plus one
  leak per failed run plus two from the battery would push later rungs
  into `6A81` for the wrong reason. Recycle (`AT+CFUN=1,1`, ~50 s, then
  **TTY re-detection** — the fixed port will not survive the shuffle,
  and the promised auto-detect has no logic) before R1a, between
  R1a/R1b, and before R2.
- **(D) Deployment-order requirement (runbook-level)**: the atcsimUC
  tarball ships the driver at the same path as the stock install, and
  upstream `a43e46ae` has no `LPAC_DRIVER_HOME` override — extracting
  UC over `/` replaces the stock driver. R1a must complete before the
  UC overlay deploys, with the order recorded in the session log (or a
  disjoint tree verified first).
- **(E) EID grep can never match** (lpac emits JSON): use
  `grep -o '"eid"[: ]*"[0-9A-Fa-f]\{32\}"'`.

Classification: review finding, not a lane dispute — same class as the
rehearsal catching the battery's verdict glob.

**Re-verification of fixes (d0bdda3, this review): AGREE.** All five
items verified in the amended script first-hand: (A)
`LPAC_APDU_AT_DEVICE` in both invocations, `LPAC_QMI_DEV` gone; (B)
`at_read` reader-backed battery with **hard stop** on `CPIN READY` and
enforced `EXPECTED_QCCID`/`EXPECTED_SLOT` when set (warn-only when
unset is the correct species-policy compromise — the private runbook
MUST set both); (C) `cfun_recycle` = `AT+CFUN=1,1` + 50 s +
AT-responsive re-detection across `/dev/ttyUSB*` with hard stop on no
port, updating the global `TTY`, called before R1a, between R1a/R1b,
and before R2; (D) binary-safe `grep -qa` guard blocks R1a if a UC
overlay is present at the known path; (E) JSON-aware EID grep. The
rehearsal-instrument evidence (`rehearse_c0_battery.py`) is regenerable
in-tree. **The session script is runbook-ready; the window now needs
only owner GO.**

### TLS verification finding — lpac HTTP leg ships unauthenticated (2026-10-25, this review; AMEND to the Route-1 preflight checklist)

Verified in the pinned source (`a43e46ae`, `driver/http/curl.c:92-93`):

```c
curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 0L);
curl_easy_setopt(curl, CURLOPT_SSL_VERIFYHOST, 0L);
```

Consequences:

- Every HTTPS exchange lpac performs — including the one-shot SM-DP+
  download that carries the activation credential — is **unauthenticated
  against active network attackers**. The GSMA-CI1-augmented
  `ca-certificates.crt` anchors only the standalone `curl` precheck
  (which verifies by default); it does nothing for lpac's own leg.
- The Route-1 checklist's §0 framing ("chain trust is irrelevant to
  lpac; the only failure mode is handshake crypto") is true as a
  description but unacceptable as a posture — it silently accepts the
  unauthenticated leg for the highest-consequence exchange in the plan.
- **Correction to this review's own record**: gate D7's "TLS
  verification never weakened (GSMA root bundle present)" was stated as
  if verified — the bundle's presence was verified, but with upstream's
  verification-off default the bundle was never *load-bearing* for
  lpac. Downgraded from Verified to **Contradicted-as-implemented**;
  now enforceable via the remedy below.
- Also flagged by the OpenWrt libcurl maintainer in the #24963 thread
  (2024-09-12) — the class was known publicly; our ecosystem had not
  propagated it into the window design.

**Remedy (concrete, already in-ecosystem)**: lpac PR **#452**
("driver/http: make TLS verification configurable, add trust store") —
open upstream, and part of fildunsky's curated patch set. The window's
lpac artifacts (uqmi route and any AT-route provisioning build) must
carry the #452-class patch with **verification ON**, trust store =
the fingerprint-verified GSMA-augmented bundle. Preflight additions:

1. Binary-fingerprint the patched driver (`driver_http_curl.so`)
   for the verification-on/trust-store code path (same discipline as
   the atcsimUC fingerprints).
2. The router TLS precheck runs `curl --cacert <augmented bundle>`
   **without** `-k` — chain validation explicit, not incidental; any
   SSL error stops the window (as the checklist already says).
3. The libcurl symlink repoint is system-wide (curl CLI flips too —
   which is what makes the precheck faithful); add the revert to the
   post-window restore list.

Smaller checklist items: `lpac profile nickname` (the B5 fallback
rehearsal) writes persistent card metadata — document it as a card-NVM
write (reversible by re-set), prefer notification-process, and if used,
set an obviously-test value with documented cleanup. Credential
discipline and the §C no-retry/quote discipline are endorsed as written.

Classification: security-class review finding; write-window hard gate;
does not block the read-only milestone (no HTTP involved).

### Version-lever assessment (2026-10-25, owner question)

- **lpac**: the pinned `a43e46ae` **is upstream main** (verified: 0
  commits ahead; latest release v2.3.0). Custom builds differ only in
  backend selection/timeout patch. Nothing newer exists to adopt.
- **uqmi**: feed binary carries the required UIM APDU surface
  (`commands-uim.h` verified); A3's zero-cost help-text pre-check
  confirms the installed binary. No upgrade needed for the milestone.
- **libqmi**: feed 1.36.0 lacks UIM APDU; a self-built newer/fully
  configured libqmi would unlock lpac's in-process `qmi` backend — a
  future driver-class option, not needed while the uqmi backend covers
  read-only ops with zero custom artifacts. Deferred, not a derisk
  lever for this milestone.
- **libcurl**: the one variant lever that matters — at write time
  (SM-DP+ TLS), use the libcurl-openssl variant (feed default mbedtls
  has a documented lpac incompatibility class, openwrt/packages#24963).
  Read-only ops are unaffected (no HTTP).
- **Modem firmware**: an upgrade is **anti-derisk** — flash risk plus
  unpredictable CSIM-behavior change; the current firmware string
  already matches the v4.0-proven unit. Do not touch before the
  milestone.
- **OpenWrt (25.12.6)**: current; no OS action before the milestone.

---

## G. Answers to the standing review questions

1. **Was GET RESPONSE encoded incorrectly?** Yes — INS `2C` was
   constructed and (in the lane-side windows) transmitted; `C0` was
   never sent in any surviving artifact. The archived round-4 run
   never reached the exchange at all.
2. **What remains of "enabled-profile eUICC can't be provisioned over
   AT+CSIM"?** Unproven. Survivors (close-refusal/leak, `6121`
   decoupling, `6A82`, lpac ×3) keep it *plausible*; exhaustion-based
   and T=0-procedure explanations are unexcluded; v4.0 proves the
   transport class works on a blank card.
3. **Why did lpac initialization fail?** Mechanism Unknown (no wire
   capture). Leading hypothesis: channel exhaustion — probes leaked
   channels and lpac's own basic-form close leaks one per run on this
   firmware. Discriminator: fresh CFUN recycle + lpac with APDU debug.
4. **Can this exact firmware expose a reliable QMI UIM APDU interface?**
   The modem answers UIM open-channel/APDU read-class on the exact
   hardware (wwand-verified) — but only for the **active card**; both
   physical slots map to logical slot 1, logical slot 2 is refused.
   lpac-over-QMI cannot build on the current feed (libqmi UIM APIs
   compiled out — verified in the SDK).
5. **Can QMI reach the eUICC while its profile is enabled?** Not
   measured on this unit; the vendor's own tool drives QMI UIM on
   enabled-profile cards (class evidence). Moot for near-term (QMI
   deferred).
6. **Without selecting the wrong physical slot?** Over QMI: slot
   selection is not per-card — only the active card is reachable, so
   the failure mode is "wrong card only if the active slot is wrong"
   (it isn't; verified active = eUICC slot in the probe logs). Over
   MBIM: lpac's mapping-*write* on mismatch is the real hazard →
   SKIP_SLOT_MAPPING protocol (§D D2).
7. **Is the USB composition switch justified, safe, recoverable?**
   Justified *if* D3 passes (it is the only route to `/dev/cdc-wdm`);
   recoverable via documented `usbnet=1` + reboot revert (D6 requires
   the standalone drill); risks: ~2 min outage, re-enumeration,
   registration re-acquisition — tolerated by the runbook's window
   structure. Not "harmless": owner-gated, as the runbook already
   treats it.
8. **Must the old profile be disabled before download?** No. Download
   is ISD-R-scoped and additive; disable is a contingency of the
   enable ladder only.
9. **Safest validated provisioning path?** For the imminent window
   (credential in hand, MBIM gates unclosed): the **physical-reader
   path** (issue #459, as amended — this review's verdict). For the
   on-device appliance goal afterward: runbook v3 (lpac MBIM, read-only
   gate, notification-process first write, one-shot download, enable
   ladder) conditional on gates D2–D6.
10. **Should PR #30 stay a separate workstream?** Yes — deferred,
    re-scoped, fixed per §A.6 before any load-bearing role.
11. **Missing evidence before production-card operations?** D2
    (slot-mapping report), D3 (MBIM-on-target gate + latency bounds),
    D4 (artifact re-verification), D5 (uplink independence), D6
    (recovery drill). All card-safe.
12. **Do the other agents agree, and where do they disagree?** See §B.
    No open disagreement stands; two open Unknowns are parked with
    named discriminating checks (slot-mapping report; corrected
    battery). No lane consensus is claimed beyond what the dispute log
    records.

---

## H. Upstream alignment & related projects (owner-directed annex)

### H.1 Align with upstream — or adopt upstream as the base?

Answer asked 2026-10-25 (owner; constraint: stay on Rust). Two different
questions, two different answers:

**Protocol alignment: yes, unconditionally.** Whatever reader protocol
the future architecture speaks, it must speak the *stock* upstream
dialect so that unpatched upstream components work as compatibility
oracles:

- PR #30's forked control codes (`ATR=0x02/RESET=0x04`) must revert to
  upstream `vpcd.h` semantics (`OFF=0, ON=1, RESET=2, ATR=4`); handle
  `ON`; stop asserting swapped names in tests.
- The patched 2 s receive window (`libifdvpcd-amperstrand`) should be
  re-evaluated against upstream receive semantics — if upstream blocks
  rather than timing out, defect §A.6-2 (the 1.5 s deadline inside a
  2 s window) dissolves at the root rather than being re-tuned. Verify
  upstream `vicc_transmit`/`vicc_connect` timeout behavior on the
  actual pinned driver before any re-tuning work.

**Upstream as the code base: no.** Reasons:

1. **What upstream is** — vsmartcard's vpcd+VICC is a *virtual card
   simulator* stack (C/autotools driver, Python VICC). Our component is
   a *bridge* from a PC/SC surface to a real card seated behind a modem;
   the shared surface is the wire protocol, nothing more.
2. **What we would lose** — the Rust driver's hardware-proven hardening
   is the project's actual asset base: typed `AtOutcome` no-resend
   doctrine (v3.4), strict echo-correlated CSIM framing, log-security
   doctrine (v3.4.1–.2, fail-closed, O_NOFOLLOW), ssh-relay ownership
   chain (flock/PDEATHSIG/self-reap, v3.5.x), pcsc conformance battery
   (mock 15/15 + pcscd-lane GREEN), the lane tooling, and the v4.0
   hardware evidence trail. None of that exists upstream; a rebase
   resets all of it.
3. **Deployment shape** — std-only Rust cdylib with zero runtime deps
   fits the router target; the upstream C stack brings autotools,
   Python, and a different ops story to OpenWrt.
4. **Where upstream belongs** — as the *reference and oracle*: exact
   constants, framing, response semantics, test vectors, and a stock
   `libifdvpcd` running in the lane to prove interop (replacing the
  patched fork as the compatibility baseline).

### H.2 Related projects studied for this audit (beyond those already cited)

| Project | What it contributes to this audit |
|---|---|
| `frankmorgner/vsmartcard` | authoritative vpcd protocol (already cited §A.6); also the model for "virtual reader + remote card" topology decisions |
| `frankmorgner/pcsc-relay` (same author) | the simpler sibling of vpcd: relay a card to a PC/SC reader slot over a socket. If the future remote-card need is point-to-point, this class is less machinery than vpcd+agent; worth a design comparison before committing to Option E/F |
| `osmocom/osmo-remsim` | the most mature remote-smartcard architecture in OSS: client/bankd/server split, explicit card-ownership sessions, reconnect/resubscribe semantics, control/data plane separation. For *multi-host, multi-reader, long-lived* remote card use it is a stronger design reference than vpcd (which assumes one VICC per reader slot). Any Option-F design should steal its ownership/session model rather than reinvent it |
| `osmocom/pysim` | the driver README's pySim-shell soak evidence; also the reference for AT-transport APDU handling conventions (CSIM length semantics shared with our strict-framing parser) |
| `estkme-group/lpac` | provisioning semantics + the three APDU backends (already cited §A.3/A.5); the `at` backend's CSIM conventions corroborate our framing analysis |
| `ddimension/wwand` | native QMI/MBIM/AT access with on-device eSIM management (already cited §A.5); production-tested on the exact router; the post-window tooling candidate and the Option-F spike substrate (PR #33) |
| Quectel `quectel_lpad` (vendor, not OSS) | class evidence that the vendor's own eSIM channel is low-level-UIM-only, in non-ECM composition — the independent leg behind the composition-pivot reasoning |
| `Amperstrand/ccid-firmware-rs` | sibling project; its `crates/ccid-service/src/vpcd.rs` is the in-family vpcd reference the PR #30 codec was checked against — and the origin of the forked control-code convention (§A.6-3) that must now revert to upstream |

Also on record: the APDU_decrypt vendor ticket (§B delivery 3) as the
route to convert the `QSIMCFG="APDU_decrypt"` semantics from Unknown to
measured — it may yet explain the CSIM mediation observations (§A.4)
without any further card probing.

- Lane archive: probe scripts + stamped logs (2026-10-24/25 window),
  GSMA root bundle, runbook — lane-side, referenced by the findings
  doc; species-level only in this repository.
- `findings/csim-session-state` @ `a46625c` (corrected findings doc),
  `efb60ef`, `3dd188a`, `80583a2`, `d17e872` (history).
- `master` @ `3d850af`; PR #30 head `90f1e3f` (diff base `3d850af`).
- lpac upstream `estkme-group/lpac` @ `a43e46ae`: `driver/apdu/
  at_etsi_csim.c`, `driver/apdu/mbim.c` (+ `mbim_helpers.c` timeout
  facts), `driver/apdu/qmi.c`.
- wwand `ddimension/wwand` `docs/STATUS.md` (state 2026-10-24;
  exact-modem slot/SEND_APDU measurements 2026-09-26).
- vsmartcard `frankmorgner/vsmartcard` `virtualsmartcard/src/vpcd/
  vpcd.h` (control codes).
- Cross-build SDK libqmi 1.36.0 headers (UIM APDU surface compiled
  out); built artifacts sha256 `19d6f87d…aa94` / `7307e038…d45b`
  (re-hashed on disk this session).


### PR #34 final state (2026-10-25)

Pre-merge review (w19:pK): species-level sweep of the full +3472-line
diff CLEAN (credentials absent; sha256s and public test identifiers
only). The one pre-merge finding — a ~30% flake in the pin suite — was
root-caused by this review to a cross-test LUN collision (parallel
`create_by_name` swapping the shared `LUN_TX` registry entry mid-flight,
routing the APDU to the other test's identical mock: API assertions
pass, per-mock counter reads 0). Fixed by per-test LUN ownership
(`LUN_TX_NULLPTR`), mechanism documented in-code; **10/10 deterministic
green verified independently by both parties**. Verdict: APPROVE-clean
both parties. Meta-note accepted for the record: the registry-isolation
test covered multi-reader semantics but not constant-sharing between
sibling tests — the mechanism comment is the guard for future authors.
Owner merges on judgment.

### T-B resolution + PR#34 species catch (2026-10-25, closeout)

**T-B: EXECUTED-ABORTED-RESOLVED.** The owner's 20-second physical
power cut recovered the wedged modem (the VBUS-physics argument
confirmed empirically — strictly stronger than the load-switch rail
cycle); rollback completed and verified via gentle CFUN 0->1 (USB
stable — the prevention held): nadanada enabled+active, BetterRoaming
present+disabled, nothing deleted, one-shot credential untouched, toll
gate on VLAN200 throughout. A second execution attempt failed both
enables at euicc_init pre-write (card untouched) and wedged the modem
into silent-but-on-bus — both reset signatures now observed; measurement
deferred to non-reset tooling per the encoded refresh policy
(refreshFlag enable > QMI UIM power-cycle > CFUN 0->1; CFUN=1,1
forbidden after profile operations). Modem awaits a second 20 s cut
(no urgency). gpiopoke landed at tools/gpiopoke (6d00cf9) with the
verified register map — the software rung below the physical cut.

**PR#34 species catch (this review, pre-merge):** KNOWN-GOOD §8 was
appended after the pre-merge sweep and carried the rollback one-liner's
literal production ICCID — redacted to a private-runbook placeholder;
post-fix sweep confirms zero full production identifiers across the
merge set. Process lesson recorded from the straggler artifact loss:
pull session artifacts off-router before requesting power cuts (tmpfs).

Disagreement ledger at close: zero entries, all session.