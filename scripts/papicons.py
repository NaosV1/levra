#!/usr/bin/env python3
"""Generates app/Sources/Debarras/Design/Papicons.swift from PapIcons.

PapIcons (MIT, (c) 2025 PapTools, https://pub.dev/packages/papicons) ships
its icons as Dart data. Only the icons listed in ICONS are converted, into
SwiftUI paths on the icons' 24 x 24 grid: the app needs no image assets and
icons take the colour of the surrounding text.

To add an icon, find its name in the PapIcons catalogue, add it to ICONS,
download the package and run:

    scripts/papicons.py path/to/papicons/lib/styles/stroke_rounded.dart

The downloaded file is only read as data. Anything other than path
commands and numbers is rejected, so nothing from it can end up as code.
"""

import re
import sys
from pathlib import Path

VERSION = "1.0.0"
OUTPUT = Path(__file__).resolve().parent.parent / "app/Sources/Debarras/Design/Papicons.swift"

# Swift case name -> PapIcons name (without the `strokeRounded` prefix).
ICONS = {
    "activity": "Activity01",
    "alert": "Alert02",
    "android": "Android",
    "arrowDown": "ArrowDown01",
    "arrowLeft": "ArrowLeft01",
    "arrowRight": "ArrowRight01",
    "arrowUpRight": "ArrowUpRight01",
    "cancel": "Cancel01",
    "check": "Tick02",
    "checkCircle": "CheckmarkCircle02",
    "chip": "Chip",
    "clean": "Clean",
    "clock": "Clock01",
    "commandLine": "CommandLine",
    "copy": "Copy01",
    "cube": "Cube",
    "delete": "Delete02",
    "download": "Download04",
    "feather": "Feather",
    "file": "File01",
    "folder": "Folder01",
    "folderOpen": "FolderOpen",
    "gitBranch": "GitBranch",
    "grid": "GridView",
    "hardDrive": "HardDrive",
    "info": "InformationCircle",
    "layers": "Layers01",
    "loading": "Loading03",
    "lock": "SquareLock02",
    "package": "Package",
    "refresh": "Refresh",
    "search": "Search01",
    "settings": "Settings02",
    "smartphone": "SmartPhone01",
    "sourceCode": "SourceCode",
    "stop": "StopCircle",
    "system": "ComputerSettings",
    "user": "UserCircle",
}

TOKEN = re.compile(r"[MLCHVZ]|-?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?")
SEPARATORS = re.compile(r"[\s,]*")


class UnsupportedIcon(Exception):
    pass


def blocks(source):
    pattern = re.compile(r"static const List<List<dynamic>> strokeRounded(\w+) = \[(.*?)\n  \];", re.S)
    return dict(pattern.findall(source))


def elements(block):
    for kind, body in re.findall(r"\[\s*'(\w+)',\s*\{(.*?)\}\s*\]", block, re.S):
        yield kind, dict(re.findall(r"'(\w+)':\s*'([^']*)'", body))


def number(text):
    value = float(text)
    return f"{value:.4f}".rstrip("0").rstrip(".") or "0"


def point(x, y):
    return f"CGPoint(x: {number(x)}, y: {number(y)})"


def tokens(d):
    found = TOKEN.findall(d)
    # Everything between the tokens must be separators: no stray text.
    leftover = TOKEN.sub(" ", d)
    if SEPARATORS.fullmatch(leftover) is None:
        raise UnsupportedIcon(f"unexpected characters in path data: {d!r}")
    return found


def path_statements(d):
    """Absolute M, L, H, V, C and Z commands, the only ones the icons use."""
    items = tokens(d)
    statements = []
    index, command = 0, None
    current = (0.0, 0.0)

    def take(count):
        nonlocal index
        values = items[index:index + count]
        if len(values) != count or any(re.fullmatch(r"[A-Z]", v) for v in values):
            raise UnsupportedIcon(f"truncated {command} command in {d!r}")
        index += count
        return [float(v) for v in values]

    while index < len(items):
        if re.fullmatch(r"[A-Z]", items[index]):
            command = items[index]
            index += 1
        elif command is None:
            raise UnsupportedIcon(f"path data must start with a command: {d!r}")
        if command == "M":
            x, y = take(2)
            statements.append(f"path.move(to: {point(x, y)})")
            current = (x, y)
            command = "L"  # extra coordinate pairs after M are lines
        elif command == "L":
            x, y = take(2)
            statements.append(f"path.addLine(to: {point(x, y)})")
            current = (x, y)
        elif command == "H":
            (x,) = take(1)
            statements.append(f"path.addLine(to: {point(x, current[1])})")
            current = (x, current[1])
        elif command == "V":
            (y,) = take(1)
            statements.append(f"path.addLine(to: {point(current[0], y)})")
            current = (current[0], y)
        elif command == "C":
            x1, y1, x2, y2, x, y = take(6)
            statements.append(
                f"path.addCurve(to: {point(x, y)}, control1: {point(x1, y1)}, control2: {point(x2, y2)})"
            )
            current = (x, y)
        elif command == "Z":
            statements.append("path.closeSubpath()")
            command = None
        else:
            raise UnsupportedIcon(f"unsupported path command {command}")
    return statements


def icon_layers(name, block):
    """Statements grouped by stroke width: outlines are 1.5 wide, dots are
    drawn as tiny segments with a thicker round stroke (1.8 or 2)."""
    layers = {}
    for kind, attributes in elements(block):
        if attributes.get("fill", "none") != "none":
            raise UnsupportedIcon(f"{name}: filled shapes are not supported")
        width = number(attributes.get("strokeWidth", ""))
        statements = layers.setdefault(width, [])
        if kind == "path":
            statements += path_statements(attributes["d"])
        elif kind == "circle":
            cx, cy, r = (float(attributes[key]) for key in ("cx", "cy", "r"))
            statements.append(
                f"path.addEllipse(in: CGRect(x: {number(cx - r)}, y: {number(cy - r)}, "
                f"width: {number(2 * r)}, height: {number(2 * r)}))"
            )
        else:
            raise UnsupportedIcon(f"{name}: unsupported element {kind}")
    if not layers:
        raise UnsupportedIcon(f"{name}: no drawable element")
    return sorted(layers.items(), key=lambda layer: float(layer[0]))


def generate(source):
    available = blocks(source)
    lines = [
        f"// Generated by scripts/papicons.py from PapIcons {VERSION}. Do not edit by hand.",
        "// PapIcons: MIT License, Copyright (c) 2025 PapTools.",
        "",
        "import SwiftUI",
        "",
        "/// The PapIcons used by the app, as outlines on their 24 × 24 grid.",
        "enum Papicon: CaseIterable, Sendable {",
    ]
    lines += [f"    case {case}" for case in sorted(ICONS)]
    lines += [
        "",
        "    /// Round strokes grouped by width on the grid: 1.5 for outlines,",
        "    /// thicker for the short segments drawn as dots.",
        "    var strokes: [(width: CGFloat, path: Path)] {",
        "        switch self {",
    ]
    for case in sorted(ICONS):
        name = ICONS[case]
        if name not in available:
            raise UnsupportedIcon(f"no icon named {name} in PapIcons {VERSION}")
        lines += [f"        case .{case}:", "            return ["]
        for width, statements in icon_layers(name, available[name]):
            lines.append(f"                ({width}, Path {{ path in")
            lines += [f"                    {statement}" for statement in statements]
            lines.append("                }),")
        lines.append("            ]")
    lines += ["        }", "    }", "}", ""]
    return "\n".join(lines)


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    source = Path(sys.argv[1]).read_text(encoding="utf-8")
    try:
        swift = generate(source)
    except UnsupportedIcon as error:
        sys.exit(f"papicons: {error}")
    OUTPUT.write_text(swift, encoding="utf-8")
    print(f"{OUTPUT.relative_to(OUTPUT.parents[4])}: {len(ICONS)} icons")


if __name__ == "__main__":
    main()
