"""Analyze bounded lab logs without promoting EC proxies into die sensors."""
import argparse, json, math, statistics
from pathlib import Path

def rows(path):
    return [json.loads(line) for line in path.read_text(encoding="utf-8-sig").splitlines() if line.strip()]

def summary(items):
    if not items:
        return {"samples": 0}
    def stats(values):
        return {"min": min(values), "median": statistics.median(values), "max": max(values)}
    return {"samples": len(items), "ec_a_c": stats([r["sample"]["ec_celsius"][0] for r in items]),
            "ec_b_c": stats([r["sample"]["ec_celsius"][1] for r in items]),
            "fans_rpm": [stats([r["sample"]["actual_rpm"][i] for r in items]) for i in range(3)],
            "all_automatic": all(all(m[1] == 0 for m in r["sample"]["modes"]) for r in items)}

def gpu_comparison(items):
    items = [r for r in items if "nvidia" in r]
    result = {"phases": {p: summary([r for r in items if r["phase"] == p]) for p in ("baseline", "gpu_load", "recovery")}}
    if not items:
        return result
    for channel in range(2):
        ec = [r["sample"]["ec_celsius"][channel] for r in items]
        nv = [r["nvidia"]["celsius"] for r in items]
        mx, my = statistics.mean(ec), statistics.mean(nv)
        denom = math.sqrt(sum((v-mx)**2 for v in ec)*sum((v-my)**2 for v in nv))
        result[f"ec_{'ab'[channel]}_vs_nvidia"] = {
            "mean_absolute_difference_c": statistics.mean(abs(a-b) for a, b in zip(ec, nv)),
            "max_absolute_difference_c": max(abs(a-b) for a, b in zip(ec, nv)),
            "correlation": sum((a-mx)*(b-my) for a, b in zip(ec, nv))/denom if denom else None,
            "note": "Sequential provider samples; correlation alone does not identify sensor location or latency."}
    result["nvidia_utilization_max_percent"] = max(r["nvidia"]["utilization_percent"] for r in items)
    result["nvidia_temperature_range_c"] = [min(r["nvidia"]["celsius"] for r in items), max(r["nvidia"]["celsius"] for r in items)]
    result["max_provider_query_window_ms"] = max(r["utc_unix_ms"]-r["thermal_captured_unix_ms"] for r in items)
    return result

def analyze(directory):
    return {"cpu": {phase: summary(rows(directory/f"{phase}.jsonl")) for phase in ("baseline", "cpu-load", "cpu-recovery")},
            "gpu": gpu_comparison(rows(directory/"gpu-response.jsonl")) if (directory/"gpu-response.jsonl").exists() else None,
            "cpu_die_reference_available": False, "physical_fan_mapping_verified": False,
            "sensor_roles_promoted": False,
            "limits": "EC readings remain proxies. This test never changes fan targets, curves, voltages or performance modes. Physical mapping and CPU die reference are separate requirements."}

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = analyze(args.directory)
    with args.output.open("x", encoding="utf-8") as file:
        json.dump(result, file, indent=2)
    print(json.dumps(result, indent=2))
