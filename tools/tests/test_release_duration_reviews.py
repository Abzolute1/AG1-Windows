import importlib.util
import json
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace

ROOT = Path.cwd()
HERE = ROOT / "tools/release_duration"
sys.path[:0] = [str(HERE), str(ROOT / "tools/release_duration"), str(ROOT / "tools")]
from release_gate import ref
from review_checks import reject_failed_reviews
spec = importlib.util.spec_from_file_location("candidate_duration", HERE / "run_duration.py")
duration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(duration)

class Reviews(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / ".local", prefix="p14-review-control-")
        self.out = Path(self.temp.name)
        (self.out / "claim-reviews").mkdir()
        (self.out / "records.json").write_text("[]")
        (self.out / "verification.json").write_text('{"status":"passed"}')
        self.task = {"index":0, "records":ref(self.out / "records.json"), "verification":ref(self.out / "verification.json")}
        self.review = {"task_index":0, "status":"failed", "reviewer":"Codex", "rationale":"Actual retained source violates the retrieved contract.", "records":self.task["records"], "verification":self.task["verification"]}
        self.path = self.out / "claim-reviews/000000.json"
    def tearDown(self):
        self.temp.cleanup()
    def publish(self):
        staged = self.path.with_suffix(".pending")
        staged.write_text(json.dumps(self.review))
        staged.replace(self.path)
    def test_absent_partial_and_passed_reviews_do_not_create_acceptance(self):
        reject_failed_reviews(self.out, [self.task])
        self.path.write_text('{"status":')
        reject_failed_reviews(self.out, [self.task])
        self.review["status"] = "passed"
        self.publish()
        reject_failed_reviews(self.out, [self.task])
        self.assertFalse((self.out / "accepted-duration.json").exists())
    def test_bound_failure_prevents_the_next_actual_activity_callback(self):
        self.publish()
        runner = duration.Run.__new__(duration.Run)
        runner.out, runner.tasks = self.out, [self.task]
        called = []
        with self.assertRaisesRegex(RuntimeError, "Quality review failed"):
            runner.activity("task", lambda _: called.append(True))
        self.assertEqual(called, [])
        self.assertFalse((self.out / "activities").exists())
    def test_failed_review_for_another_task_is_not_silently_used(self):
        self.review["task_index"] = 1
        self.publish()
        with self.assertRaisesRegex(RuntimeError, "does not bind"):
            reject_failed_reviews(self.out, [self.task])
    def test_changed_retained_records_fail_integrity(self):
        self.publish()
        (self.out / "records.json").write_text('["changed"]')
        with self.assertRaises(Exception):
            reject_failed_reviews(self.out, [self.task])
    def test_review_symlink_is_rejected(self):
        target = self.out / "elsewhere.json"
        target.write_text(json.dumps(self.review))
        self.path.symlink_to(target)
        with self.assertRaisesRegex(RuntimeError, "symlink"):
            reject_failed_reviews(self.out, [self.task])
    def test_operator_interrupt_records_failed_execution_and_drains(self):
        runner = duration.Run.__new__(duration.Run)
        runner.out = self.out
        runner.args = SimpleNamespace(hours=8)
        runner.package_sha = "0" * 64
        runner.seed = {"token_count":2500000}
        runner.bound = []
        runner.activities, runner.scenarios, runner.tasks, runner.commands = [], [], [], []
        runner.client = object()
        stopped = threading.Event()
        (self.out / "memory.jsonl").write_text("")
        runner.sampler = SimpleNamespace(start=lambda: None, stopped=stopped, close=stopped.set, samples=[])
        drained = []
        def close():
            drained.append(True)
            runner.client = None
        def interrupt(*_):
            raise KeyboardInterrupt()
        runner.close = close
        runner.activity = interrupt
        with self.assertRaisesRegex(RuntimeError, "KeyboardInterrupt"):
            runner.run()
        report = json.loads((self.out / "duration-result.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["error"], "KeyboardInterrupt()")
        self.assertFalse(report["release_acceptance"])
        self.assertEqual(drained, [True])
        self.assertTrue(stopped.is_set())

if __name__ == "__main__":
    unittest.main(verbosity=2)
