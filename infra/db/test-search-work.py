import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('search_work', Path(__file__).with_name('compare-search-work.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class BudgetTest(unittest.TestCase):
    def fixture(self, rows, blocks=100, work=1000):
        return {
            'rows': rows, 'recent_rows': 20000, 'queries': 1,
            'api_samples': [{'name': name, 'concurrency': n, 'roundtrip_ms': 1}
                            for name in module.API for n in (1, 8) for _ in range(n * 30)],
            'samples': [{'name': 'messages-common', 'concurrency': n, 'blocks': blocks,
                         'relations': {'messages': work}, 'roundtrip_ms': 1}
                        for n in (1, 8) for _ in range(n * 30)],
        }

    def compare(self, a, b):
        with tempfile.TemporaryDirectory() as folder:
            files = [Path(folder) / str(i) for i in range(2)]
            for file, data in zip(files, [a, b]):
                file.write_text(json.dumps(data))
            with contextlib.redirect_stdout(io.StringIO()):
                module.compare(*files)

    def test_bounded_work_passes(self):
        self.compare(self.fixture(300000), self.fixture(1000000, 110, 1050))

    def test_linear_buffers_fail(self):
        with self.assertRaises(AssertionError):
            self.compare(self.fixture(300000), self.fixture(1000000, 334))

    def test_linear_scan_fails(self):
        with self.assertRaises(AssertionError):
            self.compare(self.fixture(300000), self.fixture(1000000, 110, 3340))

    def test_missing_concurrent_samples_fail(self):
        for key in ('samples', 'api_samples'):
            data = self.fixture(1000000)
            data[key].pop()
            with self.assertRaises(AssertionError):
                self.compare(self.fixture(300000), data)


if __name__ == '__main__':
    unittest.main()
