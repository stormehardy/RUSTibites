#!/usr/bin/env python3
"""visual_diff.py - A/B visual parity harness for captured UI sessions.

For every captured runtime UI session, renders the viewer's exact-session UI
(`rustibites-view --ui-session NAME --ui-only --screenshot ... --width 1920 --height 1080`, or a precomputed
render from --renders-dir) and compares it with the game reference overlay/full frame in
assets/game/reference/functional_ui. Metrics: alpha-mask IoU, RGB MAE on jointly-opaque pixels, exact-pixel %,
alpha bbox offsets, and a diff heatmap. Writes deterministic CSV, worst-first markdown leaderboard, JSON and
diffs. No game launches are performed by this harness.
"""
from __future__ import annotations

import argparse
import csv
import gzip
import json
import re
import shlex
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_REF = ROOT / "assets/game/reference/functional_ui"
DEFAULT_VIEWER = ROOT / "target/release/rustibites-view.exe"
DEFAULT_OUT = ROOT / "target/ui_visual_diff"
DEFAULT_WORLD = ROOT / "testdata/parity/default/rust_seed_600.zip"
WIDTH, HEIGHT = 1920, 1080


def load_rgba(path: Path) -> np.ndarray:
    img = Image.open(path).convert("RGBA")
    if img.size != (WIDTH, HEIGHT):
        img = img.resize((WIDTH, HEIGHT), Image.LANCZOS)
    return np.asarray(img, dtype=np.uint8)


def alpha_bbox(mask: np.ndarray):
    ys, xs = np.nonzero(mask)
    if len(xs) == 0:
        return None
    return [int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max())]


def ref_for(session: str, refdir: Path, mapping: dict[str, str] | None,
            native_dir: Path | None = None):
    """Reference lookup: native root first, then legacy root, then mapping.

    Native captures are authoritative; legacy refs are known 320x240 hidden-
    window upscales and are returned with ``native=False`` so the harness can
    flag them instead of silently treating blur as a viewer regression.
    """
    if native_dir is not None:
        native_dir = Path(native_dir).resolve()
    # Explicit native map wins over legacy session-named refs.
    if mapping and session in mapping:
        mapped = Path(mapping[session])
        if mapped.is_file():
            native = native_dir is not None and native_dir in mapped.resolve().parents
            return mapped, "mapped", native
    for root, native in ((native_dir, True), (refdir, False)):
        if root is None:
            continue
        root = Path(root)
        for suffix, kind in (("_overlay_1920x1080.png", "overlay"),
                             ("_full_1920x1080.png", "full"),
                             ("_1920x1080.png", "frame")):
            p = root / f"{session}{suffix}"
            if p.is_file():
                return p, kind, native
    return None, None, None


def sessions_from_runtime(path: Path) -> list[str]:
    if not path.is_file():
        return []
    try:
        data = json.loads(gzip.open(path, "rt", encoding="utf-8").read())
        return sorted((data.get("sessions") or {}).keys())
    except Exception:
        return []


def discover_sessions(args) -> list[str]:
    if args.only:
        names = list(args.only)
    elif args.sessions_file:
        doc = json.loads(Path(args.sessions_file).read_text(encoding="utf-8"))
        names = list(doc) if isinstance(doc, list) else list(doc.get("sessions") or doc.keys())
    else:
        names = sessions_from_runtime(ROOT / "assets/game/ui_runtime.json.gz")
        names += sorted(p.name[:-len("_overlay_1920x1080.png")] for p in args.reference_dir.glob("*_overlay_1920x1080.png"))
        names += sorted(p.name[:-len("_full_1920x1080.png")] for p in args.reference_dir.glob("*_full_1920x1080.png"))
        names = sorted(set(names))
    if args.glob:
        pat = re.compile(args.glob)
        names = [n for n in names if pat.search(n)]
    return sorted(dict.fromkeys(names))[: args.limit] if args.limit else sorted(dict.fromkeys(names))


def render_one(args, session: str, out_png: Path) -> tuple[bool, str]:
    if args.renders_dir:
        for suffix in (".png", "_overlay_1920x1080.png"):
            p = args.renders_dir / f"{session}{suffix}"
            if p.is_file():
                Image.open(p).convert("RGBA").save(out_png)
                try:
                    rel = p.resolve().relative_to(ROOT)
                except ValueError:
                    rel = p
                return True, f"renders-dir:{rel.as_posix()}"
        return False, f"missing render {args.renders_dir / (session + '.png')}"
    viewer = Path(args.viewer)
    if not viewer.is_file():
        return False, f"viewer not found: {viewer}"
    cmd = [str(viewer)]
    if args.world:
        cmd.append(args.world)
    cmd += shlex.split(args.viewer_args or "")
    cmd += ["--ui-session", session]
    if not args.viewer_compose:
        cmd.append("--ui-only")
    cmd += ["--screenshot", str(out_png), "--width", str(WIDTH), "--height", str(HEIGHT)]
    try:
        proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, timeout=args.timeout)
    except subprocess.TimeoutExpired:
        return False, f"viewer timeout after {args.timeout}s"
    if proc.returncode != 0 or not out_png.is_file():
        tail = (proc.stderr or proc.stdout or "").strip().splitlines()[-3:]
        return False, f"viewer exit {proc.returncode}: {' | '.join(tail)}"
    return True, " ".join(cmd)


def reference_quality(path: Path, native: bool, sharpness_min: float = 1.0) -> dict:
    """Flag legacy/upscaled references so blur is not scored as viewer error.

    ``ref_sharpness`` is the mean absolute grayscale neighbour difference; a
    native 1920 text/UI capture is materially sharper than the old 320x240
    upscales. The flag is advisory by itself; ``--require-native-references``
    makes it a gate.
    """
    arr = np.asarray(Image.open(path).convert("RGBA"), dtype=np.uint8)
    gray = arr[:, :, :3].astype(np.float32).mean(axis=2)
    gx = float(np.abs(np.diff(gray, axis=1)).mean()) if gray.shape[1] > 1 else 0.0
    gy = float(np.abs(np.diff(gray, axis=0)).mean()) if gray.shape[0] > 1 else 0.0
    sharpness = round((gx + gy) * 0.5, 4)
    alpha_fraction = float((arr[:, :, 3] > 0).sum()) / float(arr.shape[0] * arr.shape[1])
    # Transparent UI-only overlays have a low raw edge mean simply because most
    # pixels are empty; normalize by covered area before judging blur.
    sharpness_normalized = round(sharpness / max(alpha_fraction, 0.05), 4)
    suspect = (not native) or sharpness_normalized < sharpness_min
    return {
        "native_reference": native,
        "pixel_authoritative": native and not suspect,
        "ref_sharpness": sharpness,
        "ref_sharpness_normalized": sharpness_normalized,
        "ref_alpha_fraction": round(alpha_fraction, 6),
        "rescale_blur_suspect": suspect,
    }


def flatten_background(arr: np.ndarray, color: tuple[int, int, int]) -> np.ndarray:
    """Composite an RGBA render over an opaque capture background (UI-on-black)."""
    alpha = arr[:, :, 3].astype(np.float32) / 255.0
    rgb = arr[:, :, :3].astype(np.float32)
    out = np.empty((arr.shape[0], arr.shape[1], 4), dtype=np.uint8)
    out[:, :, :3] = (
        rgb * alpha[:, :, None] + np.asarray(color, dtype=np.float32) * (1.0 - alpha[:, :, None])
    ).round().astype(np.uint8)
    out[:, :, 3] = 255
    return out


def image_diagnostics(arr: np.ndarray) -> dict:
    """Alpha/color/edge diagnostics used to separate font/SDF blur from geometry."""
    alpha = arr[:, :, 3] > 0
    rgb = arr[:, :, :3]
    gray = rgb.astype(np.float32).mean(axis=2)
    gx = np.abs(np.diff(gray, axis=1))
    gy = np.abs(np.diff(gray, axis=0))
    edge = np.zeros_like(gray, dtype=bool)
    edge[:, :-1] |= gx > 8.0
    edge[:-1, :] |= gy > 8.0
    if alpha.any():
        packed = (rgb[:, :, 0].astype(np.uint32) << 16) | (rgb[:, :, 1].astype(np.uint32) << 8) | rgb[:, :, 2]
        unique_colors = int(np.unique(packed[alpha]).size)
    else:
        unique_colors = 0
    return {
        "alpha_pixels": int(alpha.sum()),
        "alpha_bbox": alpha_bbox(alpha),
        "unique_opaque_colors": unique_colors,
        "edge_pixels": int(edge.sum()),
        "edge_density": round(float(edge.sum()) / float(edge.size), 6),
        "sharpness": round(float((gx.mean() if gx.size else 0.0) + (gy.mean() if gy.size else 0.0)) * 0.5, 4),
    }


def diff_metrics(ref: np.ndarray, ren: np.ndarray):
    am = ref[:, :, 3] > 0
    bm = ren[:, :, 3] > 0
    union = am | bm
    inter = am & bm
    iou = float(inter.sum() / union.sum()) if union.any() else 1.0
    both = inter
    if not both.any():
        both = union
    if both.any():
        dr = ref[:, :, :3][both].astype(np.int16) - ren[:, :, :3][both].astype(np.int16)
        mae = float(np.abs(dr).mean())
        exact = int((dr == 0).all(axis=1).sum())
        exact_pct = 100.0 * exact / int(both.sum())
    else:
        mae, exact_pct = 0.0, 100.0
    rb, nb = alpha_bbox(am), alpha_bbox(bm)
    offsets = None
    if rb and nb:
        offsets = {"dx0": nb[0] - rb[0], "dy0": nb[1] - rb[1], "dx1": nb[2] - rb[2], "dy1": nb[3] - rb[3],
                   "dwidth": (nb[2] - nb[0]) - (rb[2] - rb[0]), "dheight": (nb[3] - nb[1]) - (rb[3] - rb[1])}
    # deterministic heatmap: amplified absolute RGB delta on union, red where alpha masks differ
    rgb = np.abs(ref[:, :, :3].astype(np.int16) - ren[:, :, :3].astype(np.int16)).astype(np.uint8)
    heat = np.zeros((HEIGHT, WIDTH, 3), dtype=np.uint8)
    if union.any():
        heat[union] = np.clip(rgb[union].astype(np.int16) * 4, 0, 255).astype(np.uint8)
    only = np.zeros_like(union)
    only[am & ~bm] = True
    only[bm & ~am] = True
    heat[only] = [255, 0, 0]
    return {"alpha_iou": iou, "rgb_mae_opaque": mae, "exact_pixel_pct": exact_pct,
            "exact_pixels": exact, "opaque_compared_pixels": int(both.sum()),
            "alpha_pixels_ref": int(am.sum()), "alpha_pixels_render": int(bm.sum()),
            "bbox_ref": rb, "bbox_render": nb, "bbox_offset": offsets}, heat


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--reference-dir", default=str(DEFAULT_REF), help="legacy reference root")
    ap.add_argument("--native-reference-dir", default=None,
                    help="authoritative native-1920 reference root (tried before legacy refs)")
    ap.add_argument("--ref-sharpness-min", type=float, default=1.0,
                    help="flag refs below this grayscale sharpness as rescale-blur suspects")
    ap.add_argument("--viewer-compose", action="store_true",
                    help="let the viewer compose a background (omit --ui-only) for full-frame menu A/B")
    ap.add_argument("--render-background", default=None,
                    help="flatten renders over black/white/#RRGGBB for UI-on-black menu refs")
    ap.add_argument("--diagnostics", action="store_true",
                    help="record alpha/color/edge diagnostics (font/TMP blur separation)")
    ap.add_argument("--require-native-references", action="store_true",
                    help="fail if any session uses a legacy/blur-suspect reference")
    ap.add_argument("--viewer", default=str(DEFAULT_VIEWER))
    ap.add_argument("--world", default=str(DEFAULT_WORLD), help="world path passed to the viewer before UI flags")
    ap.add_argument("--viewer-args", default="", help="extra viewer flags inserted before --ui-session")
    ap.add_argument("--renders-dir", default=None, help="skip viewer; use DIR/<session>.png")
    ap.add_argument("--reference-map", default=None, help="JSON session->reference PNG path")
    ap.add_argument("--sessions-file", default=None, help="JSON array of session names")
    ap.add_argument("--only", action="append", default=[], help="session name (repeatable)")
    ap.add_argument("--glob", default=None, help="regex filter over session names")
    ap.add_argument("--limit", type=int, default=None)
    ap.add_argument("--timeout", type=float, default=120.0)
    ap.add_argument("--out-dir", default=str(DEFAULT_OUT))
    ap.add_argument("--csv", default=None)
    ap.add_argument("--md", default=None)
    ap.add_argument("--json", default=None)
    ap.add_argument("--diffs-dir", default=None)
    ap.add_argument("--min-iou", type=float, default=None)
    ap.add_argument("--max-mae", type=float, default=None)
    ap.add_argument("--min-exact-pct", type=float, default=None)
    ap.add_argument("--allow-missing", action="store_true")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--self-test", action="store_true", help="run a synthetic identity/reference metric self-test")
    args = ap.parse_args()
    args.reference_dir = Path(args.reference_dir)
    args.renders_dir = Path(args.renders_dir) if args.renders_dir else None
    outdir = Path(args.out_dir)
    outdir.mkdir(parents=True, exist_ok=True)
    renders_dir = outdir / "renders"
    renders_dir.mkdir(parents=True, exist_ok=True)
    diffs_dir = Path(args.diffs_dir) if args.diffs_dir else outdir / "diffs"
    diffs_dir.mkdir(parents=True, exist_ok=True)

    if args.self_test:
        a = np.zeros((HEIGHT, WIDTH, 4), np.uint8)
        a[100:200, 100:300, :3] = [10, 20, 30]
        a[100:200, 100:300, 3] = 255
        m, _ = diff_metrics(a, a.copy())
        assert m["alpha_iou"] == 1.0 and m["rgb_mae_opaque"] == 0.0 and m["exact_pixel_pct"] == 100.0
        b = a.copy()
        b[110:210, 120:320, :3] = [11, 21, 31]
        m2, _ = diff_metrics(a, b)
        print(json.dumps({"self_test": "ok", "identity": m, "shifted": m2}, indent=2))
        return 0

    mapping = None
    if args.reference_map:
        mapping = json.loads(Path(args.reference_map).read_text(encoding="utf-8"))
        if isinstance(mapping, dict) and isinstance(mapping.get("sessions"), dict):
            mapping = mapping["sessions"]
    sessions = discover_sessions(args)
    rows, missing = [], []
    for sess in sessions:
        ref, kind, native = ref_for(sess, args.reference_dir, mapping, args.native_reference_dir)
        out_png = renders_dir / f"{sess}.png"
        if ref is None:
            missing.append({"session": sess, "error": "no reference png"})
            if not args.allow_missing:
                continue
        quality = reference_quality(ref, native, args.ref_sharpness_min) if ref else None
        plan = {"session": sess, "reference": str(ref) if ref else None, "reference_kind": kind,
                "reference_quality": quality, **(quality or {}),
                "render_png": str(out_png),
                "viewer_cmd": None if args.renders_dir else " ".join(
                    [str(args.viewer), args.world] + shlex.split(args.viewer_args or "") +
                    ["--ui-session", sess] + ([] if args.viewer_compose else ["--ui-only"]) +
                    ["--screenshot", str(out_png), "--width", str(WIDTH), "--height", str(HEIGHT)])}
        if args.dry_run:
            rows.append(plan)
            continue
        ok, how = render_one(args, sess, out_png)
        if not ok:
            rows.append({**plan, "render_ok": False, "error": how})
            continue
        if ref is None:
            rows.append({**plan, "render_ok": True, "error": "missing reference"})
            continue
        ref_arr, ren_arr = load_rgba(ref), load_rgba(out_png)
        if args.render_background:
            raw = args.render_background.strip().lower()
            color = {"black": (0, 0, 0), "white": (255, 255, 255)}.get(raw)
            if color is None:
                value = raw.removeprefix("#")
                if len(value) != 6:
                    raise SystemExit("--render-background must be black, white or #RRGGBB")
                color = (int(value[0:2], 16), int(value[2:4], 16), int(value[4:6], 16))
            ren_arr = flatten_background(ren_arr, color)
        metrics, heat = diff_metrics(ref_arr, ren_arr)
        heat_path = diffs_dir / f"{sess}.png"
        Image.fromarray(heat, "RGB").save(heat_path)
        row = {**plan, "render_ok": True, "render_source": how, **metrics,
               **(quality or {}),
               "heatmap": str(heat_path.relative_to(outdir).as_posix())}
        if args.diagnostics:
            ref_diag = image_diagnostics(ref_arr)
            render_diag = image_diagnostics(ren_arr)
            row["ref_diagnostics"] = ref_diag
            row["render_diagnostics"] = render_diag
            row["sharpness_ratio"] = (
                round(render_diag["sharpness"] / ref_diag["sharpness"], 3)
                if ref_diag["sharpness"]
                else None
            )
        if args.min_iou is not None and metrics["alpha_iou"] < args.min_iou:
            row["threshold_fail"] = f"iou<{args.min_iou}"
        if args.max_mae is not None and metrics["rgb_mae_opaque"] > args.max_mae:
            row["threshold_fail"] = (row.get("threshold_fail", "") + f" mae>{args.max_mae}").strip()
        if args.min_exact_pct is not None and metrics["exact_pixel_pct"] < args.min_exact_pct:
            row["threshold_fail"] = (row.get("threshold_fail", "") + f" exact<{args.min_exact_pct}").strip()
        rows.append(row)

    okrows = [r for r in rows if r.get("render_ok") and r.get("alpha_iou") is not None]
    aggregate = {"sessions": len(rows), "rendered": sum(1 for r in rows if r.get("render_ok")),
                 "missing_reference": len(missing),
                 "mean_alpha_iou": float(np.mean([r["alpha_iou"] for r in okrows])) if okrows else None,
                 "median_alpha_iou": float(np.median([r["alpha_iou"] for r in okrows])) if okrows else None,
                 "mean_rgb_mae_opaque": float(np.mean([r["rgb_mae_opaque"] for r in okrows])) if okrows else None,
                 "mean_exact_pixel_pct": float(np.mean([r["exact_pixel_pct"] for r in okrows])) if okrows else None,
                 "threshold_failures": sum(1 for r in rows if r.get("threshold_fail")),
                 "legacy_references": sum(1 for r in rows if r.get("native_reference") is False),
                 "rescale_blur_suspects": sum(1 for r in rows if r.get("rescale_blur_suspect"))}
    config = {"reference_dir": str(args.reference_dir.relative_to(ROOT) if args.reference_dir.is_relative_to(ROOT) else args.reference_dir),
              "viewer": str(args.viewer), "world": str(args.world), "viewer_args": args.viewer_args,
              "sessions": len(sessions), "width": WIDTH, "height": HEIGHT,
              "render_background": args.render_background, "viewer_compose": args.viewer_compose,
              "thresholds": {"min_iou": args.min_iou, "max_mae": args.max_mae, "min_exact_pct": args.min_exact_pct}}
    doc = {"schema": "rustibites.ui.visual_diff.v1", "config": config, "aggregate": aggregate,
           "sessions": rows, "missing_references": missing}
    json_path = Path(args.json) if args.json else outdir / "results.json"
    csv_path = Path(args.csv) if args.csv else outdir / "results.csv"
    md_path = Path(args.md) if args.md else outdir / "leaderboard.md"
    fields = ["session", "reference_kind", "native_reference", "pixel_authoritative",
              "ref_sharpness", "rescale_blur_suspect", "render_ok", "error", "alpha_iou", "rgb_mae_opaque",
              "sharpness_ratio", "ref_diagnostics", "render_diagnostics",
              "exact_pixel_pct", "exact_pixels", "opaque_compared_pixels", "alpha_pixels_ref", "alpha_pixels_render",
              "bbox_ref", "bbox_render", "bbox_offset", "threshold_fail", "reference", "render_source"]
    with csv_path.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=fields, extrasaction="ignore")
        w.writeheader()
        for r in rows:
            w.writerow({k: (json.dumps(r.get(k)) if isinstance(r.get(k), (dict, list)) else r.get(k, "")) for k in fields})
    ranked = sorted(okrows, key=lambda r: (r["alpha_iou"], -r["rgb_mae_opaque"]))
    lines = ["# UI visual parity leaderboard (worst first)", "",
             f"- sessions: {aggregate['sessions']}, rendered: {aggregate['rendered']}, missing refs: {aggregate['missing_reference']}",
             f"- mean alpha IoU: {aggregate['mean_alpha_iou']}; median: {aggregate['median_alpha_iou']}",
             f"- mean RGB MAE (joint opaque): {aggregate['mean_rgb_mae_opaque']}; mean exact-pixel %: {aggregate['mean_exact_pixel_pct']}",
             f"- legacy refs: {aggregate.get('legacy_references', 0)}; rescale-blur suspects: {aggregate.get('rescale_blur_suspects', 0)}",
             "", "| # | session | alpha IoU | RGB MAE | exact % | bbox d(x0,y0,x1,y1) | ref | threshold |",
             "|---|---|---|---|---|---|---|---|"]
    for i, r in enumerate(ranked, 1):
        off = r.get("bbox_offset") or {}
        offs = f"{off.get('dx0')},{off.get('dy0')},{off.get('dx1')},{off.get('dy1')}" if off else "-"
        ref_flag = "native" if r.get("pixel_authoritative") else ("legacy" if r.get("native_reference") is False else "suspect")
        lines.append(f"| {i} | {r['session']} | {r['alpha_iou']:.4f} | {r['rgb_mae_opaque']:.3f} | "
                     f"{r['exact_pixel_pct']:.2f} | {offs} | {ref_flag} | {r.get('threshold_fail', '')} |")
    md_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    json_path.write_text(json.dumps(doc, indent=2), encoding="utf-8")
    print(json.dumps({"out_dir": str(outdir), "json": str(json_path), "csv": str(csv_path), "md": str(md_path),
                      "aggregate": aggregate}, indent=2))
    failed = any(r.get("threshold_fail") or r.get("render_ok") is False for r in rows)
    if args.require_native_references and any(r.get("rescale_blur_suspect") for r in rows):
        failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
