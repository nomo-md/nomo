// 全时间段流式统计。固定 4096 桶，误差上界为值的 1% + 0.01 单位，不丢弃早期或后期样本。
const BASE = 1.01, SIZE = 4096;
export class Distribution {
  constructor() { this.bins = new Float64Array(SIZE); this.n = 0; this.min = Infinity; this.max = -Infinity; this.overflow = 0; }
  add(value, count = 1) {
    if (!Number.isFinite(value) || value < 0 || !Number.isFinite(count) || count <= 0) return;
    const index = value === 0 ? 0 : Math.max(1, Math.ceil(Math.log(value + 1) / Math.log(BASE)));
    if (index >= SIZE) this.overflow += count;
    this.bins[Math.min(SIZE - 1, index)] += count; this.n += count;
    this.min = Math.min(this.min, value); this.max = Math.max(this.max, value);
  }
  merge(summary) {
    if (!summary?.n) return;
    for (const [index, count] of summary.bins) this.bins[index] += count;
    this.n += summary.n; this.min = Math.min(this.min, summary.min); this.max = Math.max(this.max, summary.max); this.overflow += summary.overflow ?? 0;
  }
  summary() {
    const q = ratio => { let sum = 0; for (let i = 0; i < SIZE; i++) { sum += this.bins[i]; if (sum >= Math.ceil(this.n * ratio)) return i === SIZE - 1 && this.overflow ? null : Math.min(this.max, Math.max(this.min, BASE ** i - 1)); } return null; };
    return { n: this.n, min: this.n ? this.min : null, median: this.n ? q(.5) : null, max: this.n ? this.max : null,
      p95: this.n >= 20 ? q(.95) : null, p99: this.n >= 100 ? q(.99) : null,
      method: 'full-span-log-histogram', relativeError: .01, absoluteError: .01, overflow: this.overflow,
      bins: [...this.bins.entries()].filter(([, count]) => count) };
  }
}

export function samplingQuality(metrics, intervalMs, policy) {
  const reasons = [];
  if (!metrics || metrics.samples < policy.minimumSamples) reasons.push('有效进程采样不足');
  if ((metrics?.privateBytes?.n ?? 0) < policy.minimumSamples || (metrics?.cpuMachinePercent?.n ?? 0) < 2) reasons.push('CPU 或私有提交指标缺失');
  if (!Number.isFinite(metrics?.meanSamplerCostMs) || metrics.meanSamplerCostMs > intervalMs * policy.maxMeanSamplerFraction) reasons.push('主采样开销超出预算');
  if ((metrics?.maxGapMs ?? Infinity) > intervalMs * policy.maxGapIntervals) reasons.push('采样有过长间隔');
  if ((metrics?.coverage ?? 0) < policy.minimumCoverage) reasons.push('采样覆盖率不足');
  return { status: reasons.length ? 'invalid' : 'valid', reasons, policy };
}

export function baselineEligibility(report) {
  if (report.status !== 'passed' || report.configuration !== 'release' || report.config?.suites?.includes('diagnostics')) return '只允许通过的非诊断 Release 结果';
  if (!report.cases?.length || report.cases.some(c => c.dataQuality?.status !== 'valid')) return '采样质量未通过，不能建立性能基线';
  if (report.cases.some(c => (c.requiredMetrics ?? []).some(key => !Number.isFinite(c.performanceMetrics?.[key])))) return '必需性能指标缺失';
  return null;
}
