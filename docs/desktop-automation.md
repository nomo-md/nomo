# 原生桌面测试

现有测试运行器位于 [test-lab](../test-lab/README.md)，使用独立 `automation` feature 构建 Rust/GPUI 测试版。产品构建不需要 Node.js，普通发布不启用该 feature。

运行器保留旧版样本校验、Windows 采样、真实键盘、统计和报告；控制协议升级为原生版 3，不能与旧 WebView 构建或旧性能基线混用。

```powershell
node scripts/automation/run.mjs build
node scripts/automation/run.mjs --dataset original --config test-lab/suites/quick.json --no-build
node scripts/automation/run.mjs --config test-lab/suites/release.json --no-build
```

样本需先导入并通过原始清单的哈希校验。运行会占用隔离测试窗口的前台，真实键盘失焦会明确失败。具体接口、指标定义、可用场景及尚未覆盖的验收项以 test-lab 说明和每次 `coverage.json` 为准，构建或功能断言通过不能替代桌面与发布验收。
