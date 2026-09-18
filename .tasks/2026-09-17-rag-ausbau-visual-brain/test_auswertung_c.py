import copy
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('auswertung_c', Path(__file__).with_name('auswertung-c.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def fixture(first, count, suite_cases=254):
    metric = {'hit': True, 'source_recall': 1.0, 'reciprocal_rank': 1.0, 'relevant_candidates': 1,
              'missing_context_terms': [], 'forbidden_context_terms': [], 'grounded_selection_possible': True,
              'expected_source_lost_to_relevance': False, 'answer_terms_lost_to_relevance': False}
    result = {'config': {'fixture': True}, 'corpus_root': '/fixture', 'corpus_hash': 'fixture-corpus',
              'golden_hash': 'fixture-golden', 'chunks': 3, 'html_sources': 3, 'embedding_fingerprint': 'fixture-embedder',
              'reranker_fingerprint': 'fixture-reranker', 'rounds': 1, 'suite_cases': suite_cases, 'generation_calls': 0,
              'latency_scope': 'fixture', 'quality_scope': 'fixture', 'cases': count, 'range_start': first,
              'range_end': first + count, 'timestamp_unix': 0, 'rss_kib': 1, 'peak_rss_kib': 1,
              'model_load_ms': 1.0, 'index_ms': 1.0,
              'case_results': [{'suite_position': i + 1, 'question': 'Fixture ' + str(i), 'answerable': True,
                               **{mode: copy.deepcopy(metric) for mode in ['bm25', 'hybrid', 'reranked']}}
                              for i in range(first, first + count)]}
    for mode in ['bm25', 'hybrid', 'reranked', 'rerank']:
        result[mode + '_samples_ms'] = [1.0] * count
    return result


class MergeTests(unittest.TestCase):
    def merge(self, *parts):
        with tempfile.TemporaryDirectory() as directory:
            paths = []
            for index, part in enumerate(parts):
                path = Path(directory) / (str(index) + '.json')
                path.write_text(json.dumps(part))
                paths.append(path)
            return module.merge(paths)

    def test_vollstaendige_suite_wird_einmal_aggregiert(self):
        result = self.merge(fixture(0, 127), fixture(127, 127))
        self.assertTrue(result['complete_suite'])
        self.assertEqual(result['bm25']['query_samples'], 254)
        self.assertEqual(result['reranked']['hit_cases'], 254)

    def test_luecken_werden_abgelehnt(self):
        with self.assertRaises(ValueError):
            self.merge(fixture(0, 127))

    def test_doppelte_bereiche_werden_abgelehnt(self):
        with self.assertRaises(ValueError):
            self.merge(fixture(0, 127), fixture(0, 127))

    def test_verschiedene_korpora_werden_abgelehnt(self):
        second = fixture(127, 127)
        second['corpus_hash'] = 'anderer-fixture-korpus'
        with self.assertRaises(ValueError):
            self.merge(fixture(0, 127), second)


if __name__ == '__main__':
    unittest.main()
