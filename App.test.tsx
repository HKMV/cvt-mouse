// `nsh test` 在无头引擎里打开这个 App：界面起得来、几张卡的标题都在，就算过了。
// 曲线本身的行为在 Rust 侧（`cargo test`，8 个用例）。
import { expect, open, test } from "nsh/test";

test("renders the settings page", async () => {
  const app = await open("./App.tsx");
  const texts = app.texts();
  expect(texts).toContain("CvtMouse 滚轮变速");
  expect(texts).toContain("变速曲线");
  expect(texts).toContain("按应用");
  expect(texts).toContain("统计");
});

test("统计条默认收起，点「展开」才出现明细", async () => {
  const app = await open("./App.tsx");
  // 收起时只有一行累计量：明细里的字段名不在树上。
  expect(app.texts()).toContain("展开");
  expect(app.texts()).not.toContain("原样放行");

  await app.press("text:展开");

  expect(app.texts()).toContain("原样放行");
  expect(app.texts()).toContain("注入失败");
  expect(app.texts()).toContain("收起");
  expect(app.texts()).not.toContain("展开");
});
