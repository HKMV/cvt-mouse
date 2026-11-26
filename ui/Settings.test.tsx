// `nsh test` 在无头引擎里打开这个 App：界面起得来、几张卡的标题都在，就算过了。
// 曲线本身的行为在 Rust 侧（`cargo test`，8 个用例）。
//
// 视口按真实窗口开（`nsh.json` 的 880×760）：默认的手机视口（390 宽）会把
// 「日志级别」这一行左侧的标题挤成 0 宽，测出来像是控件没了。
import { expect, open, test } from "nsh/test";

const WINDOW = { width: 880, height: 760 };

test("renders the settings page", async () => {
  const app = await open("../App.tsx", WINDOW);
  const texts = app.texts();
  expect(texts).toContain("CvtMouse 鼠标无级变速");
  // 定位是「鼠标无级变速」，但只有滚轮实现了：界面上要写清楚，别让人以为指针也能变。
  expect(texts).toContain("当前版本实现的是滚轮；指针（光标移动）的无级变速在计划里。");
  expect(texts).toContain("变速曲线");
  expect(texts).toContain("按应用");
  expect(texts).toContain("统计");
  // 日志级别可配：界面和文件各一组，四个档次。
  expect(texts).toContain("日志级别");
  expect(texts).toContain("界面");
  expect(texts).toContain("文件");
  expect(texts).toContain("调试");
});

test("统计条默认收起，点「展开」才出现明细", async () => {
  const app = await open("../App.tsx", WINDOW);
  // 收起时只有一行累计量：明细里的字段名不在树上。
  expect(app.texts()).toContain("展开");
  expect(app.texts()).not.toContain("原样放行");

  await app.press("text:展开");

  expect(app.texts()).toContain("原样放行");
  expect(app.texts()).toContain("注入失败");
  expect(app.texts()).toContain("收起");
  expect(app.texts()).not.toContain("展开");
});


