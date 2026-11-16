# Gate retro: invite-lounge-erkennung after round four

Branch `invite-lounge-erkennung`, base `origin/main` 92d9654c. Rounds 1 to 4
judged four successive branch heads, all BLOCK. The branch has since been
squashed into one commit, so the per-round heads and the commits of the
round-five fixer pass are no longer reachable and are named by their round
below.

## 1. Why four rounds

| Round | Head | Content findings | Hook lines | Causes (ledger) |
|---|---|---|---|---|
| 1 | round 1 head | 6 | 7 | spec 2, untested 4, instance 1 |
| 2 | round 2 head | 8 | 1 | regression 1, instance 3, spec 1 |
| 3 | round 3 head | 18 | 13 | regression 2, instance 2, untested 2, spec 1 |
| 4 | round 4 head | 4 new, 8 STILL, 9 judged FALSE | 0 | regression 1, instance 2, untested 1 |

- New findings: round 1 introduced four new request arms (wish, conditional,
  typo tolerance, access term) without a negative twin for any of them.
- Incomplete fixes: every round fixed the named example by a word list change
  (blockers, per-form exclusions, an access context word). The next round
  found the sibling that the list did not name: braucht/brauchte, then
  brauch/rauche; dich/jemanden, then dir/jemand/verteilen; wissen, then
  gewusst/erfahren.
- Regressions: round 2 widened is_offer and dropped real requests with their
  friend code. Round 3 unified word lists and added wäre to the question
  signal ("Die Beta wäre mir zu stressig" became Room). Round 4 replaced the
  edit distance with a bräuch prefix that took every person of the verb and
  reached Direct for an offerer.
- Drift: four copies of the subjunctive forms, three of the addressees and two
  of the invite nouns disagreed (wäre, einer). Each unification changed
  decisions that nobody measured.
- Root of all four: the rules were word co-occurrence on the whole message
  with no subject and no complement. The export measurement with the real
  Rust code only came in round 4.

## 2. Open findings triaged

| Id | Verdict | Reason |
|---|---|---|
| #36 | REAL, first half fixed; second half outside delta | own_offer_to_someone is gone since the round 4 head; "Ich suche jemanden mit Invite, Code 123456789" is Room (fixture `suche_nach_jemandem_mit_invite_bleibt_anfrage`). The second half is the pre-existing is_offer twin, see #39. |
| #39 | outside delta | is_offer is logically identical to origin/main (SECOND_PERSON_OBJECTS holds the same dich, dir, euch set as the base literals) and the branch adds no call to it. Pre-existing twin; the fix goes to the OFFENE-URSACHEN entry for is_offer. |
| #40 | REAL, fixed | Round-five fixer pass: wish arm needs a receive or play verb or an invite noun as last word and no other person in the clause. All five examples are None fixtures. |
| #41 | REAL, fixed | Round-five fixer pass: "Würde gerne wissen, ob ..." has no positive complement; "Könnte jemand erklären, wie man ..." has no self request. Both are None fixtures. |
| #42 | REAL, fixed | Round-five fixer pass: brauche and benötige need an invite complement in a clause without another person. "... brauche nur etwas Geduld" is a None fixture. |
| #43 | REAL, fixed | Round-five fixer pass: IMPERSONAL_REQUEST_RE builds its modals from CAN_FORMS, COULD_FORMS and MAY_FORMS and its nouns and verbs from INVITE_STEMS, INVITATION_NOUN and RECEIVE_VERBS. INVITE_TERM_RE is back to the three base stems, so "Wie nehme ich eine Freundeseinladung an?" stays None as at base. |
| #46 | REAL, fixed | close_word on brauche is gone since the round 4 head; need forms are whole tokens. "Ich hab noch Einladungen, falls wer eine brauch" is a None fixture. |
| #47 | REAL, fixed | Round-five fixer pass: "wissen" and "gewusst" are no positive complement. Both examples are None fixtures. |
| #54 | REAL, fixed | Round-five fixer pass: bräuchte is a whole token, the clause must not name another person and an ich in another clause does not rescue it. Both examples are None fixtures; the Direct example is also in `neue_arme_erreichen_direct_nur_mit_ich_und_invite`. |
| #55 | REAL, fixed | Round-five fixer pass: WISH_BLOCKERS is gone; the closed other-person class and the positive complement decide. All three examples are None fixtures. |
| #56 | REAL, fixed | Round-five fixer pass: "gewusst" and "erfahren" fall out without being listed. Both examples are None fixtures. |
| #57 | REAL, fixed | Round-five fixer pass: one constant per form class; the wish or need clause alone gives Room, so "Wäre gerne beim Playtest dabei" and "Würde gerne beim Playtest mitmachen" are both Room. The question signal keeps its ten base words, so "Die Beta wäre mir zu stressig" and "Ein Invite für mich wäre cool" stay None. |
| hook error_path (request_kind) | REAL, fixed | Round-five fixer pass: every regex request_kind reads is `Option` and fails closed; `detektor_ausdruecke_sind_gueltig` proves both patterns compile. |
| hook stale_state (request_kind, is_offer, invite_verb, object_before_invite, need_verb, wish_request, access_term, is_invite_request, has_question_signal, has_friend_code) | TRIVIA | The units invite_verb, object_before_invite, need_verb and access_term no longer exist. The others are pure functions of the message text with no stored state, and the diff touches no persisted struct or KV key. The gate state lists no open mechanical finding. |

## 3. Fix plan for REAL findings

Done in the round-five fixer pass:

1. Need and wish arms are evaluated per clause (newline, or punctuation
   followed by whitespace or the end). A closing parenthesis or an emoji
   followed by whitespace only ends a segment, which bounds the need verb and
   the wish complement but never cuts a subject off its verb.
2. A clause that names another person (closed pronoun and addressee class,
   including dein* and euer*) gives no clause request; no rescue by an ich in
   another clause.
3. Need verbs are whole tokens and need an invite complement.
4. A wish needs an invite noun in its complement and a receive verb, a play
   verb or an invite noun as the last content word ("Würde gerne Bescheid
   bekommen, wann meine Einladung kommt" is None).
5. A clause request reaches Direct only with a literal ich and an invite noun
   in that clause; otherwise a direct address gives None.
6. One constant per word class; the question signal, INVITE_TERM_RE and
   IMPERSONAL_REQUEST_RE are built from them. One fail-closed error policy for
   the detector regexes.
7. Fixtures for every example of the open findings, the six request examples
   and the edge values; verbatim channel messages added earlier on this branch
   are replaced by paraphrased rule cases, and the branch is squashed into one
   commit, so no earlier commit carries them.

Not fixed here, written to OFFENE-URSACHEN:

- #39 and the second half of #36: is_offer decides by whole-message
  co-occurrence and drops requests with their friend code; a fix needs an
  offer shape that cannot turn an offer into Direct.
- The remaining base asks words (wie, krieg, kriege, möchte, will, suche)
  still accept an ich from another clause, e.g. "Falls jemand noch einen
  Invite will, ich hab welche" is Room at base and here. A clause-local asks
  check was measured over the export and lost two real requests of origin/main
  (1566018679840444507, 1566172236749209762: the request and the "will" sit in
  different sentences), so it is not applied.

Both are one entry in claude-config `orchestrierung/OFFENE-URSACHEN.md`
(row dated 2026-11-16, Deadlock-Bots dl-community invite lounge). That file
is uncommitted in the main claude-config checkout and is not part of this
branch.

## 4. Second opinion and clean pass

Fable second opinion on the round-four triage, four items:

| Item | Fable | Our answer |
|---|---|---|
| 1. #39 is_offer outside delta | ACCEPT, on condition that the OFFENE-URSACHEN entry exists before merge | The entry is written (section 3). |
| 2. hook stale_state TRIVIA | ACCEPT; the stated reason was wrong: record_history runs once behind `seven_days_v1`, so nothing rereads history with the new rules | Reason corrected in section 2: pure functions, no persisted struct or KV key touched. |
| 3. "Der Neue bräuchte noch einen Invite" as known residual | REJECT, REAL: None at origin/main, Room on the branch through the non-question need arm; a closed verb-position rule needs no noun list | Fixed as FIX 1 below. |
| 4. clause-blind base asks words | ACCEPT, same OFFENE-URSACHEN condition as item 1 | The entry is written (section 3). |

Fable also traced a regression outside the four items, fixed as FIX 2.

FIX 1, positive subject condition. A need or wish verb counts only when its
own clause has a self subject: ich (or a typo of it starting with "ic")
directly before or after the verb, or a dropped subject, where every word
before the verb in the clause is a greeting opener (the same constants the
impersonal regex is built from) and the word after the verb is a closed
particle or an object article that cannot start a subject. A noun or name in
the subject slot gives None without any noun list. The question flag of the
need arm is gone. Fixture `bedarf_mit_fremdem_subjekt_ist_keine_anfrage`:
"Der Neue bräuchte noch einen Invite" in statement and question form, the
verb-first statement "Bräuchte der Neue noch einen Invite" and the wish
question "Würde der Neue gerne einen Invite bekommen?", all None.

FIX 2, need complement. The invite noun may stand anywhere in the segment of
the need verb, so object-first order counts once ich is the subject next to
the verb. A need clause without an invite noun counts when an earlier clause
of the same message names an invite and the need clause ends on a referring
indefinite (einen, eine, eins, welche and similar). Fixture
`bedarf_mit_ich_als_subjekt_bleibt_anfrage`: "Einen Invite brauche ich noch"
and "Hat jemand einen Invite? Ich brauche noch einen.", both Room as at
origin/main.

Export measurement with the real `request_kind` (466 messages, temporary
uncommitted harness, deleted afterwards): 93 triggers at origin/main, 99 on
the branch, 0 lost, 0 kind changes, 0 is_offer changes. New triggers, all
requests: 1560955674320703489, 1561313882901975072, 1561381470763753573,
1565380160578912268, 1570016600667725876, 1571345819624407251. Compared with
the round-five fixer pass, 1568689190370152528 is no longer a new trigger: its
wish verb follows "&" after an earlier clause with a dropped first-person
subject, and the subject condition does not carry a subject across a
coordination. It is None at origin/main as well.

Checks on the squashed head: `cargo fmt --check -p dl-community` exit 0;
`cargo-slot clippy -p dl-community --all-targets --features testing` exit 0
with no diagnostic in the invite_lounge files; `test-db-run
dlcentral_test_invitelounge -- cargo-slot test -p dl-community --lib
--features testing -- --include-ignored` 616 passed, 0 failed, 0 ignored.
