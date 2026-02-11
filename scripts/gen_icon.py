"""Generate Nano Snipper app icon — scissors in bitcoin orange."""

from PIL import Image, ImageDraw
import math
import os

ORANGE = (247, 147, 26, 255)  # #F7931A Bitcoin orange
TRANSPARENT = (0, 0, 0, 0)


def thick_line_poly(p1, p2, width):
    """Rectangle polygon for a thick line segment with square ends."""
    x1, y1 = p1
    x2, y2 = p2
    dx, dy = x2 - x1, y2 - y1
    ln = math.hypot(dx, dy)
    if ln < 0.001:
        return []
    # Perpendicular offset
    px, py = -dy / ln * width / 2, dx / ln * width / 2
    return [
        (x1 + px, y1 + py),
        (x2 + px, y2 + py),
        (x2 - px, y2 - py),
        (x1 - px, y1 - py),
    ]


def draw_scissors(draw, size, color):
    """Draw scissors icon matching reference: tilted, angular, bold."""
    s = size
    w = s * 0.068  # stroke width — bold for small sizes

    # Reference: scissors tilted ~15° right, blades up, triangular handles below.
    # Arm 1: upper-left blade → lower-right handle (smaller/pointed)
    # Arm 2: upper-right blade → lower-left handle (bigger/thumb hole)

    # Pivot (crossing point) — above center
    # px, py = s * 0.44, s * 0.48

    # Arm 1: upper-left blade → pivot → neck → lower-right handle
    arm1_blade = (s * 0.20, s * 0.06)
    arm1_neck = (s * 0.46, s * 0.56)   # just below pivot, arms close together
    arm1_handle = (s * 0.54, s * 0.64)
    # Handle 1 triangle (lower-right, smaller finger hole)
    h1_a = arm1_handle
    h1_b = (s * 0.80, s * 0.80)
    h1_c = (s * 0.58, s * 0.90)

    # Arm 2: upper-right blade → pivot → neck → lower-left handle
    arm2_blade = (s * 0.60, s * 0.04)
    arm2_neck = (s * 0.36, s * 0.56)   # close to arm1_neck at pivot
    arm2_handle = (s * 0.24, s * 0.68)
    # Handle 2 triangle (lower-left, bigger thumb hole)
    h2_a = arm2_handle
    h2_b = (s * 0.04, s * 0.82)
    h2_c = (s * 0.30, s * 0.94)

    # Draw arm shafts (blade → neck, neck → handle start)
    for segments in [
        [(arm1_blade, arm1_neck), (arm1_neck, arm1_handle)],
        [(arm2_blade, arm2_neck), (arm2_neck, arm2_handle)],
    ]:
        for tip, end in segments:
            poly = thick_line_poly(tip, end, w)
            if poly:
                draw.polygon(poly, fill=color)

    # Draw handle triangles (3 thick lines each)
    for tri in [(h1_a, h1_b, h1_c), (h2_a, h2_b, h2_c)]:
        for i in range(3):
            a, b = tri[i], tri[(i + 1) % 3]
            poly = thick_line_poly(a, b, w)
            if poly:
                draw.polygon(poly, fill=color)


def generate_icon(target_size):
    """Generate icon at target_size with 4x supersampling."""
    ss = 4
    s = target_size * ss
    img = Image.new("RGBA", (s, s), TRANSPARENT)
    draw = ImageDraw.Draw(img)
    draw_scissors(draw, s, ORANGE)
    return img.resize((target_size, target_size), Image.LANCZOS)


def main():
    sizes = [16, 24, 32, 48, 64, 128, 256]
    images = [generate_icon(s) for s in sizes]

    resources_dir = os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "resources"
    )
    os.makedirs(resources_dir, exist_ok=True)

    # ICO (multi-size)
    images[-1].save(
        os.path.join(resources_dir, "app.ico"),
        format="ICO",
        sizes=[(s, s) for s in sizes],
        append_images=images[:-1],
    )

    # 256px PNG for reference
    images[-1].save(os.path.join(resources_dir, "app_icon_256.png"))

    # 32x32 raw RGBA for tray icon
    with open(os.path.join(resources_dir, "tray_32x32.rgba"), "wb") as f:
        f.write(images[sizes.index(32)].tobytes())

    print("Generated: app.ico, app_icon_256.png, tray_32x32.rgba")


if __name__ == "__main__":
    main()
