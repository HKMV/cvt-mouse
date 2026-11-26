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
