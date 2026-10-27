import { describe, expect, it } from "vitest";
import { duration, isActive, mergeLogs, stateBadge, type RunLog } from "./pipeline";
const log = (seq: number): RunLog => ({ seq, chunk: `line ${seq}`, stream: "stdout", step_index: 0 });

describe("增量日志", () => {
  it("乱序、重复 seq 去重且保留旧对象", () => {
    const first = log(1);
    const current = [first, log(3)];
    const merged = mergeLogs(current, [log(4), log(2), log(3), log(2)]);
    expect(merged.map((l) => l.seq)).toEqual([1, 2, 3, 4]);
    expect(merged[0]).toBe(first);
    expect(current).toHaveLength(2);
    expect(mergeLogs(merged, [log(1)])).toBe(merged);
    expect(mergeLogs(merged, [])).toBe(merged);
  });
});
it("耗时覆盖未开始、运行中、完成、分钟小时及反向时间", () => {
  expect(duration(0, 0, 1000)).toBe("—");
  expect(duration(1000, 0, 4200)).toBe("3 秒");
  expect(duration(1000, 62000)).toBe("1 分 1 秒");
  expect(duration(1000, 3662000)).toBe("1 时 1 分 1 秒");
  expect(duration(2000, 1000)).toBe("0 秒");
});
it("状态徽标逐一映射且超时不同于失败", () => {
  expect(["queued", "running", "succeeded", "failed", "cancelled", "timed_out", "skipped"].map(stateBadge).map((b) => b.label))
    .toEqual(["排队中", "运行中", "成功", "失败", "已取消", "超时", "已跳过"]);
  expect(stateBadge("timed_out").variant).not.toBe(stateBadge("failed").variant);
  expect(stateBadge("future").label).toBe("future");
  expect(["queued", "running", "succeeded", "failed", "cancelled"].map(isActive)).toEqual([true, true, false, false, false]);
});
it("cancelling keeps polling until remote cleanup completes", () => {
  expect(isActive("cancelling")).toBe(true);
  expect(stateBadge("cancelling").label).toBe("正在取消");
});
