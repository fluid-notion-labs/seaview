#!/usr/bin/env python3
"""
Analyze Chrome trace file to find performance issues after sequence loading.

Usage:
    python3 scripts/analyze_trace.py trace-*.json
"""

import json
import sys
from collections import defaultdict
from pathlib import Path


def analyze_trace(trace_file):
    """Analyze trace file and report timing after loading completes."""

    print(f"Loading trace file: {trace_file}")

    with open(trace_file, 'r') as f:
        data = json.load(f)

    # Handle both formats: list directly or dict with traceEvents
    if isinstance(data, list):
        events = data
    else:
        events = data.get('traceEvents', [])

    print(f"Total events: {len(events)}")

    # Find all system events
    systems = defaultdict(list)
    loading_complete_ts = None

    # First pass: pair up B/E events and collect X events
    pending_begins = {}  # (name, tid) -> ts

    for event in events:
        name = event.get('name', '')
        ts = event.get('ts', 0)
        dur = event.get('dur', 0)
        ph = event.get('ph', '')
        tid = event.get('tid', 0)

        # Look for loading complete marker (if logged)
        if 'LOADING COMPLETE' in name or 'loading complete' in name.lower():
            loading_complete_ts = ts
            print(f"\n✓ Found loading complete at {ts/1000000:.2f}s")

        # Handle complete events (X)
        if ph == 'X' and dur > 0:
            systems[name].append({
                'ts': ts,
                'dur': dur,
                'after_loading': loading_complete_ts and ts > loading_complete_ts
            })
        # Handle begin/end pairs (B/E)
        elif ph == 'B':
            pending_begins[(name, tid)] = ts
        elif ph == 'E':
            key = (name, tid)
            if key in pending_begins:
                begin_ts = pending_begins.pop(key)
                computed_dur = ts - begin_ts
                if computed_dur > 0:
                    systems[name].append({
                        'ts': begin_ts,
                        'dur': computed_dur,
                        'after_loading': loading_complete_ts and begin_ts > loading_complete_ts
                    })

    # If we didn't find explicit loading complete, estimate it
    # Look for last mesh asset load
    if loading_complete_ts is None:
        # Find mesh asset loading events
        mesh_loads = []
        for event in events:
            name = event.get('name', '')
            if 'asset loading' in name and ('GltfLoader' in name or 'StlLoader' in name or '.stl' in name.lower() or '.glb' in name.lower() or '.gltf' in name.lower()):
                if 'seq://' in name:  # Only sequence assets, not embedded shaders
                    ts = event.get('ts', 0)
                    mesh_loads.append(ts)

        if mesh_loads:
            # Assume loading complete 5 seconds after last mesh loads
            last_load = max(mesh_loads)
            loading_complete_ts = last_load + 5_000_000  # 5 sec in microseconds
            print(f"\n⚠ Estimated loading complete at {loading_complete_ts/1000000:.2f}s (last mesh load at {last_load/1000000:.2f}s + 5s)")
        else:
            # Find spawn/entity creation events as fallback
            spawn_times = []
            for name, calls in systems.items():
                if 'spawn' in name.lower() or 'entity' in name.lower():
                    spawn_times.extend([c['ts'] for c in calls])

            if spawn_times:
                loading_complete_ts = max(spawn_times) + 5_000_000
                print(f"\n⚠ Estimated loading complete at {loading_complete_ts/1000000:.2f}s (last spawn + 5s)")
            else:
                # Last resort fallback
                loading_complete_ts = 20_000_000  # 20 seconds
                print(f"\n⚠ Estimated loading complete at 20s (fallback)")

    # Analyze systems of interest after loading
    print("\n" + "="*80)
    print("SYSTEMS AFTER LOADING COMPLETE")
    print("="*80)

    keywords = [
        'update_mesh_display',
        'handle_frame_changes',
        'visibility',
        'allocate_and_free',
        'extract_meshes',
        'prepare_meshes',
        'queue_material',
        'Mesh3d',
    ]

    for keyword in keywords:
        matching = {}
        for name, calls in systems.items():
            if keyword.lower() in name.lower():
                after_loading = [c for c in calls if c['after_loading']]
                if after_loading:
                    matching[name] = after_loading

        if matching:
            print(f"\n📊 Systems matching '{keyword}':")
            for name, calls in matching.items():
                durations = [c['dur'] for c in calls]
                if durations:
                    avg = sum(durations) / len(durations)
                    min_dur = min(durations)
                    max_dur = max(durations)
                    p95 = sorted(durations)[int(len(durations) * 0.95)] if len(durations) > 1 else max_dur

                    print(f"  {name[:70]}")
                    print(f"    Calls: {len(calls)}")
                    print(f"    Min:  {min_dur/1000:.2f}ms")
                    print(f"    Avg:  {avg/1000:.2f}ms")
                    print(f"    P95:  {p95/1000:.2f}ms")
                    print(f"    Max:  {max_dur/1000:.2f}ms")

                    # Warn if performance is bad
                    if avg > 100:  # 0.1ms threshold
                        print(f"    ⚠️  SLOW: Average {avg/1000:.2f}ms")
                    if max_dur > 10000:  # 10ms threshold
                        print(f"    🚨 SPIKE: Max {max_dur/1000:.2f}ms")

    # Find all systems that ran after loading with timing
    print("\n" + "="*80)
    print("ALL SYSTEMS AFTER LOADING (sorted by total time)")
    print("="*80)

    system_totals = []
    for name, calls in systems.items():
        after_loading = [c for c in calls if c['after_loading']]
        if after_loading:
            total_time = sum(c['dur'] for c in after_loading)
            system_totals.append((name, len(after_loading), total_time))

    # Sort by total time descending
    system_totals.sort(key=lambda x: x[2], reverse=True)

    print(f"\nTop 30 systems by cumulative time after loading:")
    for i, (name, calls, total) in enumerate(system_totals[:30], 1):
        avg = total / calls if calls > 0 else 0
        print(f"{i:3d}. {name[:60]:60s} | {calls:5d} calls | {total/1000:8.2f}ms total | {avg/1000:6.3f}ms avg")

    # Summary
    print("\n" + "="*80)
    print("SUMMARY")
    print("="*80)
    total_events_after = sum(len([c for c in calls if c['after_loading']]) for calls in systems.values())
    print(f"Total system calls after loading: {total_events_after}")
    print(f"Unique systems after loading: {len([s for s in systems.values() if any(c['after_loading'] for c in s)])}")

    # Check for the specific issue
    print("\n🔍 Checking for visibility-swap implementation:")
    found_visibility_changes = any('visibility' in name.lower() and any(c['after_loading'] for c in calls)
                                   for name, calls in systems.items())
    found_mesh_swaps = any('mesh3d' in name.lower() and 'swap' in name.lower() and any(c['after_loading'] for c in calls)
                           for name, calls in systems.items())
    found_allocate_free = any('allocate' in name.lower() and any(c['after_loading'] for c in calls)
                              for name, calls in systems.items())

    if found_visibility_changes:
        print("  ✓ Found visibility-related systems (visibility swap working)")
    else:
        print("  ⚠ No visibility systems found after loading")

    if found_allocate_free:
        print("  🚨 Found allocate/free systems after loading (BAD - should not happen)")
    else:
        print("  ✓ No allocate/free systems found after loading (GOOD)")


if __name__ == '__main__':
    if len(sys.argv) < 2:
        print("Usage: python3 scripts/analyze_trace.py trace-*.json")
        sys.exit(1)

    trace_file = Path(sys.argv[1])
    if not trace_file.exists():
        print(f"Error: Trace file not found: {trace_file}")
        sys.exit(1)

    analyze_trace(trace_file)
