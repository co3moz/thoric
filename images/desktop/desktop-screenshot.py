#!/usr/bin/env python3
"""desktop-screenshot [MAX_WIDTH] [X,Y,WIDTH,HEIGHT] [WINDOW_ID]: the desktop, or one window, as JPEG.

stdout carries the JPEG alone and errors go to stderr. The region crops the capture, the window's
when a window is named, and MAX_WIDTH then scales it down when it is wider; 0 or an empty
argument leaves either out. Exits 1 when the capture cannot be taken, 2 for invalid arguments and
3 when libX11 cannot be loaded at all: the manager, which runs its own copy of this script in
every desktop, then uses the desktop image's desktop-screenshot instead.

The pixels come from one GetImage of the root window, which is the screen as VNC shows it, or of
the window, clipped to the screen as xwd clipped it. xwd, which this replaces, splits an area by
the visual and colormap of the windows in it and reads each part through a single window of that
kind. Wine gives its windows a colormap of their own, and an installer's full-screen window with an
empty shape, which shows nothing, then read every Wine window as black: the X server blanks what
a window does not show. One read of the root or of the window has no such window in between.
"""
import ctypes
import ctypes.util
import subprocess
import sys

USAGE = "Usage: desktop-screenshot [MAX_WIDTH] [X,Y,WIDTH,HEIGHT] [WINDOW_ID]"
DISPLAY = b":0"
ALL_PLANES = ctypes.c_ulong(-1).value
Z_PIXMAP = 2
IS_VIEWABLE = 2
LSB_FIRST = 0
BAD_WINDOW = 3
BAD_MATCH = 8
# The exit status that sends the manager to the image's own desktop-screenshot.
CANNOT_RUN = 3


class CaptureError(Exception):
    """A capture that cannot be taken, said in words for stderr."""


def arguments(args):
    """MAX_WIDTH, the region as (x, y, width, height) and the window ID, each None when left out."""
    if len(args) > 3:
        raise ValueError("too many arguments")
    max_width, region, window = (list(args) + ["", "", ""])[:3]
    try:
        width = int(max_width or "0")
    except ValueError:
        raise ValueError(f"MAX_WIDTH must be a number, not {max_width!r}") from None
    if width < 0:
        raise ValueError("MAX_WIDTH must not be negative")
    area = None
    if region:
        try:
            area = tuple(int(part) for part in region.split(","))
        except ValueError:
            area = ()
        if len(area) != 4 or min(area) < 0 or 0 in area[2:]:
            raise ValueError(f"the region must be X,Y,WIDTH,HEIGHT, not {region!r}")
    window_id = None
    if window:
        try:
            window_id = int(window[2:], 16) if window.lower().startswith("0x") else int(window, 10)
        except ValueError:
            raise ValueError(f"WINDOW_ID must be an X11 window ID such as 0x3a00007, not {window!r}") from None
    return (width or None), area, window_id


def filters(max_width, region):
    """ImageMagick's options for the region and MAX_WIDTH, in that order."""
    options = []
    if region:
        x, y, width, height = region
        options += ["-crop", f"{width}x{height}+{x}+{y}", "+repage"]
    if max_width:
        options += ["-resize", f"{max_width}x>"]
    return options


def window_area(left, top, width, height, border, screen_width, screen_height):
    """The part of a window on the screen, as (x, y, width, height) from the window's own origin,
    with its border, as xwd took it; None when none of it is on the screen. left and top are where
    the window's inside begins on the root."""
    left, top = left - border, top - border
    width, height = width + 2 * border, height + 2 * border
    right, bottom = min(left + width, screen_width), min(top + height, screen_height)
    clipped_left, clipped_top = max(left, 0), max(top, 0)
    if right <= clipped_left or bottom <= clipped_top:
        return None
    return (clipped_left - left - border, clipped_top - top - border,
            right - clipped_left, bottom - clipped_top)


def to_ppm(data, width, height, bytes_per_line, bits_per_pixel, byte_order, masks):
    """A binary PPM of a ZPixmap image whose red, green and blue are whole bytes of each pixel, as
    in every depth 24 or 32 visual; anything else is refused."""
    if bits_per_pixel not in (24, 32):
        raise CaptureError(f"unsupported image of {bits_per_pixel} bits per pixel")
    step = bits_per_pixel // 8
    offsets = []
    for mask in masks:
        shift = (mask & -mask).bit_length() - 1
        if mask <= 0 or shift % 8 or mask >> shift != 0xFF:
            raise CaptureError(f"unsupported color mask {mask:#x}")
        index = shift // 8
        offsets.append(index if byte_order == LSB_FIRST else step - 1 - index)
    row = width * step
    if bytes_per_line == row:
        pixels = bytes(data[:row * height])
    else:
        pixels = b"".join(data[line * bytes_per_line:line * bytes_per_line + row] for line in range(height))
    rgb = bytearray(width * height * 3)
    for channel, offset in enumerate(offsets):
        rgb[channel::3] = pixels[offset::step]
    return b"P6\n%d %d\n255\n" % (width, height) + bytes(rgb)


class XImage(ctypes.Structure):
    _fields_ = [("width", ctypes.c_int), ("height", ctypes.c_int), ("xoffset", ctypes.c_int),
                ("format", ctypes.c_int), ("data", ctypes.c_void_p), ("byte_order", ctypes.c_int),
                ("bitmap_unit", ctypes.c_int), ("bitmap_bit_order", ctypes.c_int),
                ("bitmap_pad", ctypes.c_int), ("depth", ctypes.c_int), ("bytes_per_line", ctypes.c_int),
                ("bits_per_pixel", ctypes.c_int), ("red_mask", ctypes.c_ulong),
                ("green_mask", ctypes.c_ulong), ("blue_mask", ctypes.c_ulong)]


class XWindowAttributes(ctypes.Structure):
    _fields_ = [("x", ctypes.c_int), ("y", ctypes.c_int), ("width", ctypes.c_int), ("height", ctypes.c_int),
                ("border_width", ctypes.c_int), ("depth", ctypes.c_int), ("visual", ctypes.c_void_p),
                ("root", ctypes.c_ulong), ("class_", ctypes.c_int), ("bit_gravity", ctypes.c_int),
                ("win_gravity", ctypes.c_int), ("backing_store", ctypes.c_int),
                ("backing_planes", ctypes.c_ulong), ("backing_pixel", ctypes.c_ulong),
                ("save_under", ctypes.c_int), ("colormap", ctypes.c_ulong), ("map_installed", ctypes.c_int),
                ("map_state", ctypes.c_int), ("all_event_masks", ctypes.c_long),
                ("your_event_mask", ctypes.c_long), ("do_not_propagate_mask", ctypes.c_long),
                ("override_redirect", ctypes.c_int), ("screen", ctypes.c_void_p)]


class XErrorEvent(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("display", ctypes.c_void_p), ("resourceid", ctypes.c_ulong),
                ("serial", ctypes.c_ulong), ("error_code", ctypes.c_ubyte), ("request_code", ctypes.c_ubyte),
                ("minor_code", ctypes.c_ubyte)]


ERROR_HANDLER = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_void_p, ctypes.POINTER(XErrorEvent))


class Display:
    """The few Xlib calls a capture makes. An X error is recorded instead of ending the process, as
    Xlib's own handler would, and no call waits for the user."""

    def __init__(self, name=DISPLAY):
        lib = ctypes.CDLL(ctypes.util.find_library("X11") or "libX11.so.6")
        Window = ctypes.c_ulong
        lib.XOpenDisplay.restype = ctypes.c_void_p
        lib.XOpenDisplay.argtypes = [ctypes.c_char_p]
        lib.XSetErrorHandler.argtypes = [ERROR_HANDLER]
        lib.XDefaultScreen.argtypes = [ctypes.c_void_p]
        lib.XDefaultRootWindow.restype = Window
        lib.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
        lib.XDisplayWidth.argtypes = lib.XDisplayHeight.argtypes = [ctypes.c_void_p, ctypes.c_int]
        lib.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
        lib.XGetWindowAttributes.argtypes = [ctypes.c_void_p, Window, ctypes.POINTER(XWindowAttributes)]
        lib.XTranslateCoordinates.argtypes = [ctypes.c_void_p, Window, Window, ctypes.c_int, ctypes.c_int,
                                              ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int),
                                              ctypes.POINTER(Window)]
        lib.XGetImage.restype = ctypes.POINTER(XImage)
        lib.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int,
                                  ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int]
        self.lib = lib
        self.errors = []
        # Kept on the object: Xlib calls it for as long as the display is open.
        self.handler = ERROR_HANDLER(self.on_error)
        lib.XSetErrorHandler(self.handler)
        self.display = lib.XOpenDisplay(name)
        if not self.display:
            raise CaptureError(f"cannot open the display {name.decode()}")
        screen = lib.XDefaultScreen(self.display)
        self.root = lib.XDefaultRootWindow(self.display)
        self.screen = (lib.XDisplayWidth(self.display, screen), lib.XDisplayHeight(self.display, screen))

    def on_error(self, _display, event):
        self.errors.append(event.contents.error_code)
        return 0

    def failed(self):
        """The first X error since the last look, after the server has answered everything sent."""
        self.lib.XSync(self.display, 0)
        code = self.errors[0] if self.errors else None
        self.errors.clear()
        return code

    def window(self, window):
        """Where the window's inside begins on the root, its width, height and border, and whether
        it is viewable."""
        attributes = XWindowAttributes()
        found = self.lib.XGetWindowAttributes(self.display, window, ctypes.byref(attributes))
        code = self.failed()
        if not found or code is not None:
            raise CaptureError(f"there is no window {window:#x}" if code == BAD_WINDOW else
                               f"window {window:#x} cannot be read (X error {code})")
        left, top, child = ctypes.c_int(), ctypes.c_int(), ctypes.c_ulong()
        self.lib.XTranslateCoordinates(self.display, window, self.root, 0, 0,
                                       ctypes.byref(left), ctypes.byref(top), ctypes.byref(child))
        if self.failed() is not None:
            raise CaptureError(f"window {window:#x} closed while it was read")
        return (left.value, top.value, attributes.width, attributes.height, attributes.border_width,
                attributes.map_state == IS_VIEWABLE)

    def image(self, drawable, x, y, width, height):
        """One GetImage of the drawable as to_ppm takes it."""
        found = self.lib.XGetImage(self.display, drawable, x, y, width, height, ALL_PLANES, Z_PIXMAP)
        code = self.failed()
        if not found or code is not None:
            raise CaptureError(f"window {drawable:#x} is not viewable" if code == BAD_MATCH else
                               f"the image of {drawable:#x} cannot be read (X error {code})")
        image = found.contents
        data = ctypes.string_at(image.data, image.bytes_per_line * image.height)
        return (data, image.width, image.height, image.bytes_per_line, image.bits_per_pixel,
                image.byte_order, (image.red_mask, image.green_mask, image.blue_mask))


def capture(display, window=None):
    """The root, or the part of the window on the screen, as a PPM."""
    screen_width, screen_height = display.screen
    if window is None:
        return to_ppm(*display.image(display.root, 0, 0, screen_width, screen_height))
    left, top, width, height, border, viewable = display.window(window)
    if not viewable:
        raise CaptureError(f"window {window:#x} is not viewable")
    area = window_area(left, top, width, height, border, screen_width, screen_height)
    if area is None:
        raise CaptureError(f"window {window:#x} is off the screen")
    return to_ppm(*display.image(window, *area))


def main(argv, open_display=Display, run=subprocess.run):
    try:
        max_width, region, window = arguments(argv[1:])
    except ValueError as error:
        print(f"desktop-screenshot: {error}\n{USAGE}", file=sys.stderr)
        return 2
    try:
        display = open_display()
    except OSError as error:
        print(f"desktop-screenshot: libX11 cannot be loaded: {error}", file=sys.stderr)
        return CANNOT_RUN
    except CaptureError as error:
        print(f"desktop-screenshot: {error}", file=sys.stderr)
        return 1
    try:
        ppm = capture(display, window)
    except CaptureError as error:
        print(f"desktop-screenshot: {error}", file=sys.stderr)
        return 1
    return run(["convert", "ppm:-", *filters(max_width, region), "-quality", "80", "jpeg:-"],
               input=ppm, check=False).returncode


if __name__ == "__main__":
    sys.exit(main(sys.argv))
