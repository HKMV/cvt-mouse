// 曲线配置变了，卡片上的读数徽标和预览图的纵轴都要跟着变。
//
// 用真实后端，但自己管好配置：先把用户的配置存下来，跑断言时用已知曲线，
// 最后写回原样 —— 这个文件跑完，%APPDATA%\CvtMouse\config.json 和跑之前一样。
//
// 只断言「配置 → 曲线信号 → 图表」这一段：滑块本身的拖动在 headless 里驱动不了
// （轨道不是可 press 的视图，拖动还会被外层 `Scroll` 当成滚动）。
import { afterAll, beforeAll, expect, open, test } from "nsh/test";
import * as api from "./src/app/api";

/** 跑之前用户自己的配置，收尾时写回。 */
const original = api.boot().config;

/** 写一份曲线，然后看界面。 */
function useCurve(start: number, mid: number, high: number): void {
  const result = api.save({
    ...original,
    curve: [
      [0, 1],
      [start, 1],
      [12, mid],
      [24, high],
    ] as api.CurvePoint[],
  });
  if (!result.ok) throw new Error(`写不进测试配置：${result.error ?? ""}`);
}

beforeAll(() => {
  useCurve(6, 3.8, 8.1);
});

afterAll(() => {
  const result = api.save(original);
  if (!result.ok) throw new Error(`配置没还原：${result.error ?? ""}`);
});

test("默认曲线：读数与纵轴刻度对得上", async () => {
  const app = await open("./App.tsx");
  const texts = app.texts();
  expect(texts).toContain("6 格/秒");
  expect(texts).toContain("3.8x");
  expect(texts).toContain("8.1x");
  // 纵轴顶到 8.1，所以最高刻度是 8x。
  expect(texts).toContain("8x");
});

test("把高速倍率调到 12，纵轴刻度和读数跟着变", async () => {
  useCurve(6, 3.8, 12);
  const app = await open("./App.tsx");
  const texts = app.texts();
  expect(texts).toContain("12.0x");
  // 纵轴跟着换成 5x/10x（d3 在 [1,12] 上取的整齐步长），原来那排 2x/4x/6x/8x 没了。
  expect(texts).toContain("10x");
  expect(texts).not.toContain("8.1x");
  expect(texts).not.toContain("8x");
});

test("把起点调到 2，虚线位置跟着变", async () => {
  useCurve(2, 3.8, 8.1);
  const app = await open("./App.tsx");
  expect(app.texts()).toContain("2 格/秒");
  useCurve(6, 3.8, 8.1);
});
