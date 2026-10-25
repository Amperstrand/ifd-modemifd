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
