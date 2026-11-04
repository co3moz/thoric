"""desktop-screenshot, which every screenshot, recording and preview of a desktop runs: the desktop
is one read of the root window and a window one read of itself, whatever colormaps the windows on
the screen use, since reading parts through other windows turned every Wine window black; the
arguments and ImageMagick's options keep the form the manager and the recorder pass; and a missing
window is an error on stderr, never a wait."""

import contextlib
import importlib.machinery
import importlib.util
import io
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
DESKTOP = ROOT / "images/desktop"


def load(name, path):
    loader = importlib.machinery.SourceFileLoader(name, str(path))
    spec = importlib.util.spec_from_loader(name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


screenshot = load("desktop_screenshot", DESKTOP / "desktop-screenshot.py")

MASKS = (0xFF0000, 0x00FF00, 0x0000FF)
ROOT_WINDOW = 0x22C


def bgrx(*pixels):
    """32-bit pixels as an LSBFirst ZPixmap holds them, from (red, green, blue)."""
    return b"".join(bytes((blue, green, red, 0)) for red, green, blue in pixels)


class FakeDisplay:
    """A screen of 4x2 pixels with windows, which records every image read from it."""

    def __init__(self, windows=None):
        self.root = ROOT_WINDOW
        self.screen = (4, 2)
        self.windows = windows or {}
        self.reads = []

    def window(self, window):
        if window not in self.windows:
            raise screenshot.CaptureError(f"there is no window {window:#x}")
        return self.windows[window]

    def image(self, drawable, x, y, width, height):
        self.reads.append((drawable, x, y, width, height))
        pixels = [(10 * (drawable & 0xF), (x + column) % 256, (y + line) % 256)
                  for line in range(height) for column in range(width)]
        return bgrx(*pixels), width, height, width * 4, 32, screenshot.LSB_FIRST, MASKS


def run(*args, display=None):
    display = display or FakeDisplay()
    converted = []

    def convert(command, input, check):
        converted.append((command, input))
        return type("Completed", (), {"returncode": 0})()

    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = screenshot.main(["desktop-screenshot", *args], open_display=lambda: display, run=convert)
    return code, converted, out.getvalue(), err.getvalue(), display


class CaptureTest(unittest.TestCase):
    def test_the_desktop_is_one_read_of_the_whole_root(self):
        code, converted, out, err, display = run()
        self.assertEqual((code, out, err), (0, "", ""))
        self.assertEqual(display.reads, [(ROOT_WINDOW, 0, 0, 4, 2)])
        command, ppm = converted[0]
        self.assertEqual(command, ["convert", "ppm:-", "-quality", "80", "jpeg:-"])
        self.assertTrue(ppm.startswith(b"P6\n4 2\n255\n"))
        # Red, green and blue of the root's first pixels, in that order.
        self.assertEqual(ppm[11:17], bytes((120, 0, 0, 120, 1, 0)))

    def test_a_window_is_one_read_of_itself_clipped_to_the_screen(self):
        # Its inside begins at (2, 1) on the root, 5x3 with a border of 1: what is on the screen of a
        # 4x2 desktop is its border's left and top and the 2x1 of its inside beside them.
        display = FakeDisplay({0x3400016: (2, 1, 5, 3, 1, True)})
        code, converted, _, err, display = run("0", "", "0x3400016", display=display)
        self.assertEqual((code, err), (0, ""))
        self.assertEqual(display.reads, [(0x3400016, -1, -1, 3, 2)])
        self.assertTrue(converted[0][1].startswith(b"P6\n3 2\n255\n"))
        # A decimal ID names the same window.
        self.assertEqual(run("", "", str(0x3400016), display=FakeDisplay(display.windows))[0], 0)

    def test_a_window_that_is_gone_hidden_or_off_the_screen_is_an_error_without_an_image(self):
        windows = {0x10: (0, 0, 2, 2, 0, False), 0x11: (9, 0, 2, 2, 0, True)}
        for window, said in (("0x3400016", "there is no window 0x3400016"), ("0x10", "is not viewable"),
                             ("0x11", "is off the screen")):
            code, converted, out, err, display = run("", "", window, display=FakeDisplay(windows))
            self.assertEqual((code, converted, out), (1, [], ""), window)
            self.assertIn(said, err)
            self.assertEqual(display.reads, [])

    def test_a_desktop_whose_libx11_cannot_be_loaded_says_so_with_status_3(self):
        # The manager runs its own copy of this script in every desktop and takes the image's
        # desktop-screenshot on 3 alone, so a capture error must never be 3.
        def missing():
            raise OSError("libX11.so.6: cannot open shared object file")

        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = screenshot.main(["desktop-screenshot"], open_display=missing, run=None)
        self.assertEqual((code, out.getvalue()), (screenshot.CANNOT_RUN, ""))
        self.assertEqual(screenshot.CANNOT_RUN, 3)
        self.assertIn("libX11 cannot be loaded", err.getvalue())
        self.assertEqual(run("", "", "0x3400016")[0], 1)

    def test_the_region_crops_before_max_width_scales_down(self):
        code, converted, *_ = run("320", "40,30,200,100", "")
        self.assertEqual(code, 0)
        self.assertEqual(converted[0][0], ["convert", "ppm:-", "-crop", "200x100+40+30", "+repage",
                                           "-resize", "320x>", "-quality", "80", "jpeg:-"])
        # The recorder passes all three, the window empty, and a width of 0 scales nothing.
        self.assertEqual(run("0", "", "")[1][0][0], ["convert", "ppm:-", "-quality", "80", "jpeg:-"])

    def test_invalid_arguments_are_refused_with_the_usage(self):
        for args in (("wide",), ("-1",), ("", "1,2,3"), ("", "1,2,0,4"), ("", "a,b,c,d"), ("", "", "window"),
                     ("1", "", "", "extra")):
            code, converted, out, err, display = run(*args)
            self.assertEqual((code, converted, out, display.reads), (2, [], "", []), args)
            self.assertIn(screenshot.USAGE, err)


class AreaTest(unittest.TestCase):
    def test_a_window_on_the_screen_is_read_whole_with_its_border(self):
        self.assertEqual(screenshot.window_area(100, 120, 320, 240, 0, 1280, 720), (0, 0, 320, 240))
        self.assertEqual(screenshot.window_area(100, 120, 320, 240, 2, 1280, 720), (-2, -2, 324, 244))

    def test_the_parts_beyond_each_edge_are_left_out(self):
        self.assertEqual(screenshot.window_area(-50, -20, 320, 240, 0, 1280, 720), (50, 20, 270, 220))
        self.assertEqual(screenshot.window_area(1200, 600, 320, 240, 0, 1280, 720), (0, 0, 80, 120))
        self.assertIsNone(screenshot.window_area(1280, 0, 320, 240, 0, 1280, 720))
        self.assertIsNone(screenshot.window_area(-320, 0, 320, 240, 0, 1280, 720))


class PixelsTest(unittest.TestCase):
    PIXELS = ((255, 0, 0), (0, 255, 0), (0, 0, 255), (1, 2, 3))
    RGB = bytes((255, 0, 0, 0, 255, 0, 0, 0, 255, 1, 2, 3))

    def test_32_bit_pixels_of_either_byte_order_become_rgb(self):
        data = bgrx(*self.PIXELS)
        self.assertEqual(screenshot.to_ppm(data, 2, 2, 8, 32, screenshot.LSB_FIRST, MASKS), b"P6\n2 2\n255\n" + self.RGB)
        msb = b"".join(data[i:i + 4][::-1] for i in range(0, len(data), 4))
        self.assertEqual(screenshot.to_ppm(msb, 2, 2, 8, 32, 1, MASKS), b"P6\n2 2\n255\n" + self.RGB)

    def test_padded_lines_and_24_bit_pixels_keep_only_the_pixels(self):
        lines = [bgrx(*self.PIXELS[:2]) + b"\xee" * 8, bgrx(*self.PIXELS[2:]) + b"\xee" * 8]
        self.assertEqual(screenshot.to_ppm(b"".join(lines), 2, 2, 16, 32, 0, MASKS), b"P6\n2 2\n255\n" + self.RGB)
        packed = b"".join(bytes((b, g, r)) for r, g, b in self.PIXELS)
        self.assertEqual(screenshot.to_ppm(packed, 2, 2, 6, 24, 0, MASKS), b"P6\n2 2\n255\n" + self.RGB)

    def test_a_format_without_whole_bytes_per_color_is_refused(self):
        with self.assertRaises(screenshot.CaptureError):
            screenshot.to_ppm(b"\0" * 8, 2, 2, 4, 16, 0, (0xF800, 0x07E0, 0x001F))
        with self.assertRaises(screenshot.CaptureError):
            screenshot.to_ppm(b"\0" * 16, 2, 2, 8, 32, 0, (0x3FF00000, 0xFFC00, 0x3FF))


if __name__ == "__main__":
    unittest.main()
