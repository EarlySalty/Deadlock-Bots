# Supporter P1 slim: round-4 finding map

Findings 158 to 260 of `p1-slim-r3b-findings.txt` (gate run `gate-p1-slim-r3b.log`), each with its class and the commit that fixes it. A finding listed twice is fixed in parts by both commits.

| Class | Defect family | Commit | Findings |
|---|---|---|---|
| 1 | fix commits without a true four-layer Ursache block | 617e8234, 5d191ec8 (rewords of 0f2dd3f8, 59a0ac5f) | 167, 193, 244 |
| 2/8 | dangling links and unreadable optional files reported as absent | 756dad7f (182 finished in e05d5e31) | 158, 164, 165, 168, 169, 171, 172, 173, 174, 179, 180, 182, 184, 185, 186, 187, 188, 189, 191, 199, 200, 201, 207, 210, 213, 214, 258, 259 |
| 3 | tip paid_at time encoding and month boundary | afd65c7b | 160, 161, 162, 238, 239, 241, 242, 243, 260 |
| 4/6 | unbounded waits and cursor pagers | 09e742f6 | 159, 166, 170, 175, 176, 255, 256, 257 |
| 5 | literal bounds restated next to their constant | 22a0cc1f | 246, 247, 250, 251, 252, 254 |
| 7 | token name chains copied per bridge | 34841138 (Ursache of 0f2dd3f8 part in 617e8234) | 216, 220, 221, 222, 224, 226, 227, 234, 235, 237 |
| 9 | supporter tick: privacy gate, role failures, empty member list | 9fc9bca1 | 215, 217, 229, 230, 231, 232, 233 |
| 10 | panel text blocks, flags and placeholders restated in render, lint, reply and docs | 2f6ed231 (192, 218 and the 59a0ac5f part of 216, 224, 227, 253 in 5d191ec8) | 163, 190, 192, 194, 195, 196, 197, 198, 202, 203, 204, 205, 206, 208, 209, 211, 212, 218, 219, 223, 253 |
| 11 | /invite bridge deadline next to the Steam worst case | 70952e7b | 236 |
| 11 | invite lounge running-dispatch guard per user | e758eb06 | 245, 248, 249 |
| 11 | supporter link client never driven over HTTP | d5166e2f | 225, 228 |
| 11 | static V2 panel edit/post loops, uploads and id reader copied per panel; PATCH fake blind | b6cd2b4b | 177, 178, 181, 183 |
| 11 | billing index check by substring | e05d5e31 | 182, 240 |

Deferred causes outside this branch are rows in `OFFENE-URSACHEN-ROWS.md` next to this file.
