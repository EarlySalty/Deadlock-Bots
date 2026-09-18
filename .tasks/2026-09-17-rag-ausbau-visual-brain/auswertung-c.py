import hashlib
import json
import math
import sys
from pathlib import Path


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]


def summary(rows, mode, samples):
    positive = [row[mode] for row in rows if row['answerable']]
    negative = [row[mode] for row in rows if not row['answerable']]
    result = {
        'positive_cases': len(positive),
        'negative_cases': len(negative),
        'hit_cases': sum(row['hit'] for row in positive),
        'source_recall_at_k': sum(row['source_recall'] for row in positive) / len(positive),
        'mrr_at_k': sum(row['reciprocal_rank'] for row in positive) / len(positive),
        'positive_cases_without_relevant_candidates': sum(row['relevant_candidates'] == 0 for row in positive),
        'negative_cases_without_relevant_candidates': sum(row['relevant_candidates'] == 0 for row in negative),
        'missing_context_cases': sum(bool(row['missing_context_terms']) for row in positive),
        'forbidden_context_cases': sum(bool(row[mode]['forbidden_context_terms']) for row in rows),
        'query_samples': len(samples),
        'query_p50_ms': percentile(samples, 0.5),
        'query_p95_ms': percentile(samples, 0.95),
    }
    for field in ['grounded_selection_possible', 'expected_source_lost_to_relevance', 'answer_terms_lost_to_relevance']:
        result[field + '_cases'] = sum(row[field] is True for row in positive)
    return result


def merge(inputs):
    parts = [json.loads(path.read_text()) for path in inputs]
    if not parts:
        raise ValueError('Mindestens ein vollständiger Teilbericht ist erforderlich')
    same = ['config', 'corpus_root', 'corpus_hash', 'golden_hash', 'chunks', 'html_sources',
            'embedding_fingerprint', 'reranker_fingerprint', 'rounds', 'suite_cases', 'generation_calls',
            'latency_scope', 'quality_scope']
    for part in parts:
        if any(part[key] != parts[0][key] for key in same):
            raise ValueError('Teilberichte verwenden verschiedene Modelle, Korpora oder Messkonfigurationen')
        if part['suite_cases'] != 224 or part['generation_calls'] != 0:
            raise ValueError('Unerwartete Messbasis')
        expected = list(range(part['range_start'] + 1, part['range_end'] + 1))
        if [row['suite_position'] for row in part['case_results']] != expected or len(expected) != part['cases']:
            raise ValueError('Unvollständiger oder widersprüchlicher Teilbericht')
        for mode in ['bm25', 'hybrid', 'reranked', 'rerank']:
            samples = part[mode + '_samples_ms']
            if len(samples) != part['cases'] * part['rounds'] or not all(math.isfinite(value) and value >= 0 for value in samples):
                raise ValueError('Ungültige Latenzmessungen')
    rows = sorted([row for part in parts for row in part['case_results']], key=lambda row: row['suite_position'])
    if [row['suite_position'] for row in rows] != list(range(1, 225)) or len({row['question'] for row in rows}) != 224:
        raise ValueError('Die Teilberichte müssen jeden Golden-Fall genau einmal abdecken')
    result = {key: parts[0][key] for key in same}
    result.update({'cases': 224, 'range_start': 0, 'range_end': 224, 'complete_suite': True,
                   'case_results': rows, 'timestamp_unix': max(part['timestamp_unix'] for part in parts),
                   'rss_kib': max(part['rss_kib'] for part in parts),
                   'peak_rss_kib': max(part['peak_rss_kib'] for part in parts),
                   'parts': [{'file': path.name, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                              'range_start': part['range_start'], 'range_end': part['range_end'],
                              'model_load_ms': part['model_load_ms'], 'index_ms': part['index_ms']}
                             for path, part in zip(inputs, parts)]})
    for mode in ['bm25', 'hybrid', 'reranked', 'rerank']:
        samples = [value for part in parts for value in part[mode + '_samples_ms']]
        result[mode + '_samples_ms'] = samples
        if mode == 'rerank':
            result['rerank_p50_ms'] = percentile(samples, 0.5)
            result['rerank_p95_ms'] = percentile(samples, 0.95)
        else:
            result[mode] = summary(rows, mode, samples)
    return result


def main():
    if len(sys.argv) < 3:
        raise SystemExit('Aufruf: python3 auswertung-c.py Ergebnis.json Teilbericht.json [Teilbericht.json ...]')
    output = Path(sys.argv[1])
    result = merge([Path(value) for value in sys.argv[2:]])
    temporary = output.with_suffix('.tmp')
    temporary.write_text(json.dumps(result, indent=2, ensure_ascii=False, allow_nan=False) + '\n')
    temporary.replace(output)
    print(json.dumps({mode: result[mode] for mode in ['bm25', 'hybrid', 'reranked']}, indent=2, ensure_ascii=False))


if __name__ == '__main__':
    main()
