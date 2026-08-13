import { describe, expect, it } from "vitest";

import {
  evaluateBudgets,
  parseCliOptions,
  parsePsRows,
  percentile,
  summarizeMeasurements,
} from "./measure";

describe("performance qualification harness", () => {
  it("accepts the package-manager argument separator", () => {
    expect(parseCliOptions(["--", "--pid", "42"]).pid).toBe(42);
  });

  it("parses and totals a process tree sample", () => {
    expect(
      parsePsRows(
        [
          "100 1 0.2 40960",
          "101 100 0.1 20480",
          "200 1 9.9 99999",
        ].join("\n"),
        100,
      ),
    ).toEqual({
      cpuPercent: 0.3,
      rssMiB: 60,
      processCount: 2,
    });
  });

  it("uses an interpolated percentile and rejects empty measurements", () => {
    expect(percentile([10, 20, 30, 40], 95)).toBeCloseTo(38.5);
    expect(() => percentile([], 95)).toThrow("at least one");
  });

  it("summarizes samples and reports every exceeded budget", () => {
    const summary = summarizeMeasurements(
      [
        { atEpochMs: 1, cpuPercent: 0.1, rssMiB: 60, processCount: 2 },
        { atEpochMs: 2, cpuPercent: 0.8, rssMiB: 90, processCount: 2 },
      ],
      [100, 170],
    );

    expect(summary.cpuP95Percent).toBeCloseTo(0.765);
    expect(summary.rssPeakMiB).toBe(90);
    expect(summary.latencyP95Ms).toBeCloseTo(166.5);
    expect(
      evaluateBudgets(summary, {
        maxCpuP95Percent: 0.5,
        maxRssMiB: 80,
        maxLatencyP95Ms: 150,
      }),
    ).toEqual({
      passed: false,
      violations: [
        "CPU p95 0.765% exceeds 0.5%",
        "RSS peak 90MiB exceeds 80MiB",
        "Latency p95 166.5ms exceeds 150ms",
      ],
    });
  });
});
