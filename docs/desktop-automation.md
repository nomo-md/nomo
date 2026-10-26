# 桌面回归与性能测试

完整工具已集中到 [test-lab](../test-lab/README.md)。默认运行真实 Release 程序，支持测试集、用例、标签、JSON 配置、独立样本、性能基线及 HTML/JSON/CSV 报告。

```powershell
pnpm test:desktop --list
pnpm test:desktop --suite standard
pnpm test:desktop --suite performance --no-build
```

源码、程序、样本和结果的组织方式，以及采样口径、人工验收边界，以 `test-lab/README.md` 为准。旧 `scripts/automation/run.mjs` 保留为新运行器的兼容入口。
