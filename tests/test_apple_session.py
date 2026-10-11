"""Recovery metadata refreshes rejected cookies without bypassing image verification."""
import email.message
import io
import unittest
import urllib.error
from unittest.mock import patch
from p1401 import apple


class Response(io.BytesIO):
    def __init__(self, data=b'', cookie=None):
        super().__init__(data); self.headers = email.message.Message()
        if cookie: self.headers['Set-Cookie'] = cookie


PAYLOAD = b'AP: product\nAU: http://oscdn.apple.com/BaseSystem.dmg\nAH: hash\nAT: image-token\nCU: http://oscdn.apple.com/BaseSystem.chunklist\nCH: hash\nCT: list-token\n'


class AppleSession(unittest.TestCase):
    def test_session_uses_https_closes_response_and_parses_multiple_cookies(self):
        response = Response(cookie='other=1;session=fresh; Path=/')
        with patch.object(apple, '_open', return_value=response) as opened:
            self.assertEqual(apple.session(), 'session=fresh')
        self.assertTrue(opened.call_args.args[0].startswith('https://'))
        self.assertTrue(response.closed)

    def test_rejected_session_is_replaced_once_and_request_regenerated(self):
        calls = []
        def open_url(req, timeout):
            calls.append(req)
            if req.data is None: return Response(cookie='session=fresh; Path=/')
            if req.get_header('Cookie') == 'session=old':
                raise urllib.error.HTTPError(req.full_url, 403, 'Forbidden', {}, None)
            return Response(PAYLOAD)
        with patch.object(apple.urllib.request, 'urlopen', side_effect=open_url):
            info = apple.image_info(24, 'session=old')
        posts = [r for r in calls if r.data]
        self.assertEqual(len(posts), 2)
        self.assertEqual(posts[1].get_header('Cookie'), 'session=fresh')
        self.assertNotEqual(posts[0].data, posts[1].data)
        self.assertTrue(all(r.full_url.startswith('https://') for r in calls))
        self.assertEqual(info['board'], 'Mac-7BA5B2D9E42DDD94')

    def test_repeated_access_refusal_is_bounded_and_actionable(self):
        posts = []
        def open_url(req, timeout):
            if req.data is None: return Response(cookie='session=fresh')
            posts.append(req); raise urllib.error.HTTPError(req.full_url, 403, 'Forbidden', {}, None)
        with patch.object(apple.urllib.request, 'urlopen', side_effect=open_url):
            with self.assertRaisesRegex(apple.AppleError, 'Диск не очищен'):
                apple.image_info(24)
        self.assertEqual(len(posts), 2)

    def test_apple_asset_transport_and_resume_headers_are_preserved(self):
        for scheme in ('http', 'https'):
            with patch.object(apple, '_open', return_value=Response()) as opened:
                url = scheme + '://oscdn.apple.com/content/BaseSystem.dmg'
                apple._asset(url, 'token', {'Range': 'bytes=10-'})
            self.assertEqual(opened.call_args.args[0], url)
            self.assertEqual(opened.call_args.args[1]['Range'], 'bytes=10-')
            self.assertEqual(opened.call_args.args[1]['Cookie'], 'AssetToken=token')

    def test_non_session_error_is_not_retried_as_cookie_failure(self):
        with patch.object(apple, 'session', return_value='session=fresh'), patch.object(apple.urllib.request, 'urlopen', side_effect=urllib.error.HTTPError('https://osrecovery.apple.com/', 404, 'Not found', {}, None)) as opened:
            with self.assertRaises(apple.AppleError): apple.image_info(24)
        self.assertEqual(opened.call_count, 1)
