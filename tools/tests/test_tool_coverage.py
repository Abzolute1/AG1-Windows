import copy
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import PIN, GateError
from tool_coverage import CAPABILITIES, WINDOWS_SOURCE, selected_cases, validate_coverage


class ToolCoverageTests(unittest.TestCase):
    def setUp(self):
        self.path = 'product/crates/ag1-platform/tests/tool_contract.rs'
        self.test = 'loaded_resource_reads_match_sdk_and_grant_no_other_workspace_access'
        self.passed = {(self.path, self.test)}
        self.selected = [{'path': 'reference-test.ts', 'name': 'selected behavior', 'sha256': '1' * 64,
                          'reference_status': 'passed', 'capabilities': ['A61']}]
        row = {**self.selected[0], 'status': 'covered',
               'evidence': [{'path': self.path, 'test': self.test,
                             'assertion': 'Explicit skill-root reads match both successful SDK rows and both denials.'}]}
        self.document = {'version': 1, 'source_commit': PIN, 'capabilities': list(CAPABILITIES), 'cases': [row]}

    def test_exact_inventory_matches_immutable_catalog(self):
        rows = selected_cases()
        self.assertEqual(len(rows), 493)
        self.assertEqual(len({row['path'] for row in rows}), 25)
        skipped = [row for row in rows if row['reference_status'] != 'passed']
        self.assertEqual(len(skipped), 2)
        self.assertTrue(all(row['path'] == WINDOWS_SOURCE for row in skipped))

    def test_evidence_requires_fresh_exact_test_and_assertion(self):
        self.assertEqual(validate_coverage(self.document, self.selected, self.passed), {'selected': 1, 'covered': 1})
        for passed in [set(), {(self.path, self.test + '_other')}]:
            with self.assertRaises(GateError):
                validate_coverage(self.document, self.selected, passed)
        for field, value in [('path', '../outside.rs'), ('path', '/etc/passwd'), ('path', 'missing.rs'), ('assertion', '')]:
            bad = copy.deepcopy(self.document)
            bad['cases'][0]['evidence'][0][field] = value
            with self.assertRaises(GateError):
                validate_coverage(bad, self.selected, self.passed)

    def test_empty_missing_duplicate_unknown_and_pending_cannot_pass(self):
        row = self.document['cases'][0]
        for cases in [[], [row, row], [{**row, 'name': 'other'}], [{**row, 'status': 'pending'}], [{**row, 'evidence': []}]]:
            bad = {**self.document, 'cases': cases}
            with self.assertRaises(GateError):
                validate_coverage(bad, self.selected, self.passed)
        with self.assertRaises(GateError):
            validate_coverage(self.document, [], self.passed)

    def test_source_and_capability_identity_cannot_be_weakened(self):
        for field, value in [('source_commit', 'other'), ('capabilities', [])]:
            with self.assertRaises(GateError):
                validate_coverage({**self.document, field: value}, self.selected, self.passed)
        for field, value in [('sha256', '0' * 64), ('capabilities', ['A71'])]:
            bad = copy.deepcopy(self.document)
            bad['cases'][0][field] = value
            with self.assertRaises(GateError):
                validate_coverage(bad, self.selected, self.passed)

    def test_corrections_require_reviewable_document_and_reason(self):
        row = self.document['cases'][0]
        row['status'] = 'corrected'
        for correction in [{}, {'reason': 'Changed behavior'}, {'reason': 'Changed behavior', 'document': 'missing.md'}]:
            row['correction'] = correction
            with self.assertRaises(GateError):
                validate_coverage(self.document, self.selected, self.passed)
        row['correction'] = {'reason': 'Explicit correction with preserved raw evidence', 'document': 'product/crates/ag1-core/TOOLS.md'}
        self.assertEqual(validate_coverage(self.document, self.selected, self.passed)['corrected'], 1)

    def test_only_observed_windows_skips_can_remain_platform_pending(self):
        row = self.document['cases'][0]
        row.update(status='platform-pending', platform='windows', gate='P14', reason='Linux host cannot certify Windows inherited handles')
        with self.assertRaises(GateError):
            validate_coverage(self.document, self.selected, self.passed)
        self.selected[0].update(path=WINDOWS_SOURCE, reference_status='skipped')
        row['path'] = WINDOWS_SOURCE
        self.assertEqual(validate_coverage(self.document, self.selected, self.passed)['platform-pending'], 1)
        for field, value in [('platform', 'linux'), ('gate', None), ('reason', '')]:
            bad = copy.deepcopy(self.document)
            bad['cases'][0][field] = value
            with self.assertRaises(GateError):
                validate_coverage(bad, self.selected, self.passed)
        row['status'] = 'covered'
        with self.assertRaises(GateError):
            validate_coverage(self.document, self.selected, self.passed)


if __name__ == '__main__':
    unittest.main()
