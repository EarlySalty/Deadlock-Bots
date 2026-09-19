import json
import sys
from pathlib import Path


def summarize(path: Path) -> None:
    data = json.loads(path.read_text(encoding="utf-8"))
    metrics = data["hybrid"]
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
        hybrid = bool(row["hybrid"]["grounded_selection_possible"])
        if bm25 and not hybrid:
            lost.append(row["question"])
        elif hybrid and not bm25:
            rescued.append(row["question"])

    print("SESSION4_LOST", json.dumps(lost, ensure_ascii=False))
    print("SESSION4_RESCUED", json.dumps(rescued, ensure_ascii=False))


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("Aufruf: session4_sweep_summary.py <bericht.json>")
    summarize(Path(sys.argv[1]))
