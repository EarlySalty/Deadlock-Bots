import json
import sys
from pathlib import Path


def summarize(path: Path, mode: str = "hybrid") -> None:
    data = json.loads(path.read_text(encoding="utf-8"))
    if mode not in {"hybrid", "reranked"}:
        raise ValueError("Modus muss hybrid oder reranked sein")
    metrics = data[mode]
    keys = [
        "positive_cases",
        "negative_cases",
        "recall_at_1",
        "recall_at_3",
        "recall_at_5",
        "mrr_at_k",
        "citation_correctness",
        "retrieval_abstain_rate",
        "false_answer_rate",
        "grounded_selection_possible_cases",
        "query_p50_ms",
        "query_p95_ms",
    ]
    print(
        "SESSION4_METRICS",
        json.dumps({key: metrics[key] for key in keys}, sort_keys=True),
    )

    lost = []
    rescued = []
    for row in data["case_results"]:
        if not row["answerable"]:
            continue
        bm25 = bool(row["bm25"]["grounded_selection_possible"])
        candidate = bool(row[mode]["grounded_selection_possible"])
        if bm25 and not candidate:
            lost.append(row["question"])
        elif candidate and not bm25:
            rescued.append(row["question"])

    print("SESSION4_LOST", json.dumps(lost, ensure_ascii=False))
    print("SESSION4_RESCUED", json.dumps(rescued, ensure_ascii=False))


if __name__ == "__main__":
    if len(sys.argv) not in {2, 3}:
        raise SystemExit("Aufruf: session4_sweep_summary.py <bericht.json> [hybrid|reranked]")
    summarize(Path(sys.argv[1]), sys.argv[2] if len(sys.argv) == 3 else "hybrid")
