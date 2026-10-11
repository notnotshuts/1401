"""Keep large child output responsive and preserve completion ordering without a display."""
import ast
from pathlib import Path
import queue
import re
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

source = ast.parse((Path(__file__).parents[1] / 'p1401/linux_gui.py').read_text())
method = next(node for cls in source.body if isinstance(cls, ast.ClassDef)
              for node in cls.body if isinstance(node, ast.FunctionDef) and node.name == 'poll')
namespace = {'queue': queue, 're': re, 'json': __import__('json'), 'GREEN': '', 'RED': ''}
exec(compile(ast.fix_missing_locations(ast.Module(body=[method], type_ignores=[])), '<GUI poll>', 'exec'), namespace)

class GuiEvents(unittest.TestCase):
    def app(self):
        app = SimpleNamespace(events=queue.Queue(), operation='check', operation_output=[],
                              append_log=Mock(), after=Mock(), progress=Mock(), action_controls=[],
                              status_label=Mock(), update_gates=Mock(), save_settings=Mock(), busy=True)
        app.poll = lambda: namespace['poll'](app)
        return app

    def test_large_output_yields_then_completes_after_logs(self):
        app = self.app(); callback = Mock()
        for i in range(450): app.events.put(('log', f'line {i}\n'))
        app.events.put(('done', (0, callback)))
        app.poll()
        self.assertTrue(app.busy); self.assertEqual(app.events.qsize(), 251)
        self.assertEqual(app.append_log.call_count, 1)
        app.poll(); app.poll()
        self.assertFalse(app.busy); callback.assert_called_once_with(0)
        self.assertEqual(app.operation_output[-1], 'line 449\n')
        self.assertEqual(len(app.operation_output), 40)
        self.assertEqual(''.join(c.args[0] for c in app.append_log.call_args_list),
                         ''.join(f'line {i}\n' for i in range(450)))
        self.assertEqual(app.after.call_count, 3)

    def test_poll_is_rescheduled_after_widget_error(self):
        app = self.app(); app.events.put(('log', 'message'))
        app.append_log.side_effect = RuntimeError('widget error')
        with self.assertRaises(RuntimeError): app.poll()
        app.after.assert_called_once_with(80, app.poll)
