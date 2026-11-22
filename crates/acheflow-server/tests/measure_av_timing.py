"""Measure the loopback captures from sustained_rtmp_av_markers."""

import argparse
import bisect
import csv
import json
import math
import os
from pathlib import Path
import re
import statistics
import subprocess


PAIR_WINDOW_SECONDS = 0.7
OFFSET_P95_SECONDS = 0.080
PEAK_OFFSET_SECONDS = 0.150
DRIFT_MS_PER_MINUTE = 20
MIN_PAIRS = 6


def run(command):
    return subprocess.run(
        command, check=True, capture_output=True, text=True, timeout=150
    )


def inspect(file, ffmpeg, ffprobe):
    streams = {"video": [], "audio": []}
    packets = run([
        ffprobe, "-v", "error", "-show_packets", "-show_entries",
        "packet=codec_type,pts_time", "-of", "json", file,
    ])
    for packet in json.loads(packets.stdout)["packets"]:
        kind = packet["codec_type"]
        if kind in streams and packet.get("pts_time") not in (None, "N/A"):
            streams[kind].append(float(packet["pts_time"]))
    for timestamps in streams.values():
        timestamps.sort()
        if len(timestamps) < 2:
            raise ValueError(f"Missing packet timestamps in {file.name}")

    filters = run([
        ffmpeg, "-nostdin", "-hide_banner", "-copyts", "-i", file,
        "-vf", "blackdetect=d=0.08:pix_th=0.20",
        "-af", "silencedetect=n=-30dB:d=0.08", "-f", "null", "-",
    ]).stderr
    markers = {
        kind: [
            float(value)
            for value in re.findall(rf"{label}:\s*(-?\d+(?:\.\d+)?)", filters)
            # The filters can flush a final black/silent interval at EOF.
            if float(value) < streams[kind][-1] - 0.10
        ]
        for kind, label in (("video", "black_end"), ("audio", "silence_end"))
    }
    return streams, markers


def within(values, start, end):
    return values[bisect.bisect_left(values, start):bisect.bisect_left(values, end)]


def gaps(values):
    return [right - left for left, right in zip(values, values[1:])]


def pairs(video, audio):
    # Pulses repeat every two seconds. This search window keeps adjacent
    # pulses distinct and leaves absent or badly displaced pulses unpaired.
    remaining = iter(audio)
    sound = next(remaining, None)
    for flash in video:
        while sound is not None and sound < flash - PAIR_WINDOW_SECONDS:
            sound = next(remaining, None)
        if sound is not None and abs(sound - flash) <= PAIR_WINDOW_SECONDS:
            yield flash, sound - flash
            sound = next(remaining, None)


def milliseconds(seconds):
    return round(seconds * 1000, 2)


def summarize(capture, start, end, fps, *, qualify, drift_required=False):
    streams, markers = capture
    video = within(markers["video"], start, end)
    audio_in_window = within(markers["audio"], start, end)
    # Include nearby audio markers even if a pulse straddles an analysis boundary.
    audio = within(markers["audio"], start - PAIR_WINDOW_SECONDS, end + PAIR_WINDOW_SECONDS)
    paired = list(pairs(video, audio))
    offsets = [offset for _, offset in paired]
    absolute = sorted(map(abs, offsets))
    p95 = absolute[math.ceil(len(absolute) * 0.95) - 1] if absolute else None
    period = max(gaps([flash for flash, _ in paired]), default=None)
    drift = None
    if len(paired) >= MIN_PAIRS:
        edge = max(2, len(paired) // 5)
        elapsed = statistics.median(t for t, _ in paired[-edge:]) - statistics.median(
            t for t, _ in paired[:edge]
        )
        drift = (statistics.median(offsets[-edge:]) - statistics.median(offsets[:edge])) * 60 / elapsed
    packet_gaps = {
        kind: max(gaps(within(timestamps, start, end)), default=None)
        for kind, timestamps in streams.items()
    }
    reasons = []
    if qualify:
        # An onset can fall across one edge of a window due to its A/V offset.
        if len(paired) < MIN_PAIRS or len(paired) < 0.95 * max(len(video), len(audio_in_window) - 1):
            reasons.append("too few matched pulses")
        if p95 is None or p95 > OFFSET_P95_SECONDS:
            reasons.append("95th-percentile offset above 80 ms or unavailable")
        if not absolute or absolute[-1] > PEAK_OFFSET_SECONDS:
            reasons.append("peak offset above 150 ms or unavailable")
        if drift_required and (drift is None or abs(milliseconds(drift)) > DRIFT_MS_PER_MINUTE):
            reasons.append("drift above 20 ms/min or unavailable")
        if period is None or period > 4.1:
            reasons.append("matched pulse gap above 4.1 s or unavailable")
        for kind, limit in (("video", 3 / fps + 0.001), ("audio", 0.065)):
            if packet_gaps[kind] is None or packet_gaps[kind] > limit:
                reasons.append(f"{kind} packet gap above {milliseconds(limit)} ms or unavailable")
    return {
        "window_s": [round(start, 2), round(end, 2)],
        "video_markers": len(video),
        "audio_markers": len(audio_in_window),
        "paired": len(paired),
        "median_offset_ms": milliseconds(statistics.median(offsets)) if offsets else None,
        "p95_absolute_offset_ms": milliseconds(p95) if p95 is not None else None,
        "peak_absolute_offset_ms": milliseconds(absolute[-1]) if absolute else None,
        "drift_ms_per_minute": milliseconds(drift) if drift is not None else None,
        "longest_paired_marker_gap_s": round(period, 3) if period is not None else None,
        "first_paired_marker_s": round(paired[0][0], 3) if paired else None,
        "last_paired_marker_s": round(paired[-1][0], 3) if paired else None,
        "max_packet_gap_ms": {
            kind: milliseconds(gap) if gap is not None else None
            for kind, gap in packet_gaps.items()
        },
        "within_budget": not reasons if qualify else None,
        "findings": reasons,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--ffmpeg", default=os.environ.get("ACHEFLOW_FFMPEG", "ffmpeg"))
    parser.add_argument("--ffprobe", default="ffprobe")
    parser.add_argument("--markers-csv", type=Path, help="save each matched pulse with its signed offset")
    args = parser.parse_args()
    phases = json.loads((args.directory / "phases.json").read_text())
    fps = phases["fps"]
    vertical_fps = phases.get("vertical_fps", fps)

    def load(name):
        return inspect(args.directory / f"{name}.flv", args.ffmpeg, args.ffprobe)

    reference = load("source")
    source_jump = {}
    for kind, timestamps in reference[0].items():
        index, jump_size = max(enumerate(gaps(timestamps)), key=lambda item: item[1])
        source_jump[kind] = {
            "before_s": round(timestamps[index] - timestamps[0], 3),
            "gap_ms": milliseconds(jump_size),
        }
    reference_report = summarize(reference, 5, 110, fps, qualify=True, drift_required=True)
    expected_jump = phases["source_jump"] - phases["source_start"]
    rejoin_report = summarize(load("source-rejoin"), 3, 17, fps, qualify=True)
    if (
        not reference_report["within_budget"]
        or not rejoin_report["within_budget"]
        or any(
            value["gap_ms"] < 1000 or abs(value["before_s"] - expected_jump) > 0.25
            for value in source_jump.values()
        )
    ):
        raise SystemExit(f"Invalid source markers or missing PTS discontinuity: {reference_report}, {source_jump}")

    main_capture = load("primary")
    start = phases["source_start"]
    load_at = phases["loaded_start"]
    jump = phases["source_jump"]
    lost = phases["source_lost"]
    rejoin = phases["source_rejoin"]
    restart = phases["output_restart"]
    streaming = phases["restart_streaming"]
    end = phases["end"]
    report = {
        "fps": {"source": fps, "horizontal": fps, "vertical": vertical_fps},
        "source_reference": reference_report,
        "source_pts_jump": source_jump,
        "source_rejoin_reference": rejoin_report,
        "primary_wall_duration_s": round(end, 2),
        "primary_media_duration_s": round(
            max(ts[-1] for ts in main_capture[0].values())
            - min(ts[0] for ts in main_capture[0].values()), 2
        ),
        "primary": {
            "baseline": summarize(main_capture, start + 5, load_at - 4, fps, qualify=True, drift_required=True),
            "under_load_before_jump": summarize(main_capture, load_at + 4, jump - 3, fps, qualify=True),
            "source_pts_jump": summarize(main_capture, jump - 3, jump + 5, fps, qualify=False),
            "under_load_after_jump": summarize(main_capture, jump + 3, lost - 3, fps, qualify=True),
            "source_reconnect": summarize(main_capture, lost - 3, rejoin + 5, fps, qualify=False),
            "after_source_reconnect": summarize(main_capture, rejoin + 3, restart - 2, fps, qualify=True),
            "parallel_output_restart": summarize(main_capture, restart - 3, streaming + 5, fps, qualify=False),
            "after_parallel_restart": summarize(main_capture, streaming + 5, end - 2, fps, qualify=True),
        },
    }
    loaded = load("loaded")
    last = min(ts[-1] for ts in loaded[0].values())
    report["vertical"] = {
        "under_load_before_jump": summarize(loaded, 4, jump - load_at - 3, vertical_fps, qualify=True),
        "under_load_after_jump": summarize(
            loaded, jump - load_at + 3, lost - load_at - 3, vertical_fps, qualify=True
        ),
        "after_source_reconnect": summarize(
            loaded, rejoin - load_at + 3, min(last - 2, restart - load_at - 2), vertical_fps, qualify=True
        ),
    }
    restarted = load("restarted")
    first = min(ts[0] for ts in restarted[0].values())
    last = min(ts[-1] for ts in restarted[0].values())
    report["vertical"]["after_parallel_restart"] = summarize(
        restarted, first + 3, last - 2, vertical_fps, qualify=True
    )
    report["primary_media_wall_difference_s"] = round(
        report["primary_media_duration_s"] - end, 2
    )
    if args.markers_csv:
        with args.markers_csv.open("w", newline="") as stream:
            writer = csv.writer(stream)
            writer.writerow(("capture", "video_pts_s", "audio_pts_s", "audio_minus_video_ms"))
            for name, (_, markers) in (
                ("horizontal", main_capture),
                ("vertical", loaded),
                ("vertical_after_restart", restarted),
            ):
                for video, offset in pairs(markers["video"], markers["audio"]):
                    writer.writerow((name, round(video, 6), round(video + offset, 6), milliseconds(offset)))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
