import { execFile } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { hostname } from "node:os";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

export interface ProcessSample {
  atEpochMs: number;
  cpuPercent: number;
  rssMiB: number;
  processCount: number;
}

export interface MeasurementSummary {
  sampleCount: number;
  cpuP95Percent: number;
  rssPeakMiB: number;
  latencyP95Ms: number | null;
}

export interface PerformanceBudgets {
  maxCpuP95Percent: number;
  maxRssMiB: number;
  maxLatencyP95Ms: number;
}

interface ProcessTotals {
  cpuPercent: number;
  rssMiB: number;
  processCount: number;
}

export interface CliOptions {
  pid: number;
  warmupSeconds: number;
  durationSeconds: number;
  intervalMs: number;
  latencyFile?: string;
  output?: string;
  budgets: PerformanceBudgets;
}

export function percentile(values: readonly number[], target: number): number {
  if (values.length === 0) {
    throw new Error("Percentile requires at least one measurement");
  }
  if (!Number.isFinite(target) || target < 0 || target > 100) {
    throw new Error("Percentile target must be between 0 and 100");
  }
  const sorted = [...values].sort((left, right) => left - right);
  const index = (target / 100) * (sorted.length - 1);
  const lower = Math.floor(index);
  const upper = Math.ceil(index);
  const weight = index - lower;
  return sorted[lower] + (sorted[upper] - sorted[lower]) * weight;
}

export function parsePsRows(output: string, rootPid: number): ProcessTotals {
  const rows = output
    .trim()
    .split(/\r?\n/)
    .map((line) => line.trim().split(/\s+/).map(Number))
    .filter(
      (row): row is [number, number, number, number] =>
        row.length === 4 && row.every(Number.isFinite),
    )
    .map(([pid, parentPid, cpuPercent, rssKiB]) => ({
      pid,
      parentPid,
      cpuPercent,
      rssKiB,
    }));
  const processIds = new Set([rootPid]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const row of rows) {
      if (!processIds.has(row.pid) && processIds.has(row.parentPid)) {
        processIds.add(row.pid);
        changed = true;
      }
    }
  }
  const tree = rows.filter((row) => processIds.has(row.pid));
  if (!tree.some((row) => row.pid === rootPid)) {
    throw new Error(`Process ${rootPid} is not running`);
  }
  return {
    cpuPercent: round(tree.reduce((total, row) => total + row.cpuPercent, 0)),
    rssMiB: round(tree.reduce((total, row) => total + row.rssKiB, 0) / 1024),
    processCount: tree.length,
  };
}

export function summarizeMeasurements(
  samples: readonly ProcessSample[],
  latencyMs: readonly number[],
): MeasurementSummary {
  if (samples.length === 0) {
    throw new Error("At least one process sample is required");
  }
  return {
    sampleCount: samples.length,
    cpuP95Percent: round(percentile(samples.map((sample) => sample.cpuPercent), 95)),
    rssPeakMiB: round(Math.max(...samples.map((sample) => sample.rssMiB))),
    latencyP95Ms: latencyMs.length > 0 ? round(percentile(latencyMs, 95)) : null,
  };
}

export function evaluateBudgets(
  summary: MeasurementSummary,
  budgets: PerformanceBudgets,
): { passed: boolean; violations: string[] } {
  const violations: string[] = [];
  if (summary.cpuP95Percent > budgets.maxCpuP95Percent) {
    violations.push(
      `CPU p95 ${summary.cpuP95Percent}% exceeds ${budgets.maxCpuP95Percent}%`,
    );
  }
  if (summary.rssPeakMiB > budgets.maxRssMiB) {
    violations.push(`RSS peak ${summary.rssPeakMiB}MiB exceeds ${budgets.maxRssMiB}MiB`);
  }
  if (summary.latencyP95Ms === null) {
    violations.push("No mouse-up-to-button latency samples were supplied");
  } else if (summary.latencyP95Ms > budgets.maxLatencyP95Ms) {
    violations.push(
      `Latency p95 ${summary.latencyP95Ms}ms exceeds ${budgets.maxLatencyP95Ms}ms`,
    );
  }
  return { passed: violations.length === 0, violations };
}

async function sampleProcessTree(rootPid: number): Promise<ProcessTotals> {
  if (process.platform === "win32") {
    const script = [
      "$processes = Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId",
      "$perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Select-Object IDProcess,PercentProcessorTime,WorkingSetPrivate",
      "$processes | ForEach-Object {",
      "  $p = $_",
      "  $m = $perf | Where-Object IDProcess -eq $p.ProcessId | Select-Object -First 1",
      "  if ($m) { [PSCustomObject]@{ pid=[int]$p.ProcessId; parentPid=[int]$p.ParentProcessId; cpuPercent=[double]$m.PercentProcessorTime; rssKiB=[double]$m.WorkingSetPrivate / 1KB } }",
      "} | ConvertTo-Json -Compress",
    ].join("; ");
    const { stdout } = await execFileAsync("powershell.exe", [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      script,
    ]);
    const parsed = JSON.parse(stdout) as
      | { pid: number; parentPid: number; cpuPercent: number; rssKiB: number }
      | Array<{ pid: number; parentPid: number; cpuPercent: number; rssKiB: number }>;
    const rows = (Array.isArray(parsed) ? parsed : [parsed])
      .map(
        (row) =>
          `${row.pid} ${row.parentPid} ${row.cpuPercent} ${row.rssKiB}`,
      )
      .join("\n");
    return parsePsRows(rows, rootPid);
  }

  const { stdout } = await execFileAsync("ps", [
    "-axo",
    "pid=,ppid=,%cpu=,rss=",
  ]);
  return parsePsRows(stdout, rootPid);
}

async function loadLatencySamples(path: string | undefined): Promise<number[]> {
  if (!path) {
    return [];
  }
  const value = JSON.parse(await readFile(path, "utf8")) as unknown;
  if (
    !Array.isArray(value) ||
    value.some((sample) => typeof sample !== "number" || !Number.isFinite(sample) || sample < 0)
  ) {
    throw new Error("Latency file must contain an array of finite non-negative milliseconds");
  }
  return value;
}

export function parseCliOptions(arguments_: readonly string[]): CliOptions {
  if (arguments_[0] === "--") {
    arguments_ = arguments_.slice(1);
  }
  const values = new Map<string, string>();
  for (let index = 0; index < arguments_.length; index += 2) {
    const name = arguments_[index];
    const value = arguments_[index + 1];
    if (!name?.startsWith("--") || value === undefined) {
      throw new Error(`Invalid argument near ${name ?? "<end>"}`);
    }
    values.set(name.slice(2), value);
  }
  const numberValue = (name: string, fallback?: number) => {
    const raw = values.get(name);
    const value = raw === undefined ? fallback : Number(raw);
    if (value === undefined || !Number.isFinite(value) || value <= 0) {
      throw new Error(`--${name} must be a positive number`);
    }
    return value;
  };
  return {
    pid: Math.trunc(numberValue("pid")),
    warmupSeconds: numberValue("warmup-seconds", 300),
    durationSeconds: numberValue("duration-seconds", 600),
    intervalMs: numberValue("interval-ms", 1_000),
    latencyFile: values.get("latency-file"),
    output: values.get("output"),
    budgets: {
      maxCpuP95Percent: numberValue("max-cpu-p95-percent", 0.5),
      maxRssMiB: numberValue("max-rss-mib", 80),
      maxLatencyP95Ms: numberValue("max-latency-p95-ms", 150),
    },
  };
}

async function run(options: CliOptions) {
  await delay(options.warmupSeconds * 1_000);
  const samples: ProcessSample[] = [];
  const deadline = Date.now() + options.durationSeconds * 1_000;
  while (Date.now() < deadline) {
    const totals = await sampleProcessTree(options.pid);
    samples.push({ atEpochMs: Date.now(), ...totals });
    await delay(options.intervalMs);
  }
  const latencyMs = await loadLatencySamples(options.latencyFile);
  const summary = summarizeMeasurements(samples, latencyMs);
  const evaluation = evaluateBudgets(summary, options.budgets);
  const report = {
    schemaVersion: 1,
    host: hostname(),
    platform: process.platform,
    architecture: process.arch,
    rootPid: options.pid,
    warmupSeconds: options.warmupSeconds,
    durationSeconds: options.durationSeconds,
    intervalMs: options.intervalMs,
    wakeupObservation: "source-and-profiler-review-required",
    budgets: options.budgets,
    summary,
    evaluation,
    samples,
  };
  const json = `${JSON.stringify(report, null, 2)}\n`;
  if (options.output) {
    await writeFile(options.output, json, "utf8");
  } else {
    process.stdout.write(json);
  }
  if (!evaluation.passed) {
    process.exitCode = 1;
  }
}

function round(value: number): number {
  return Math.round(value * 1_000) / 1_000;
}

function delay(milliseconds: number) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

const entryPath = process.argv[1];
if (entryPath && import.meta.url === pathToFileURL(entryPath).href) {
  run(parseCliOptions(process.argv.slice(2))).catch((error: unknown) => {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
    process.exitCode = 1;
  });
}
