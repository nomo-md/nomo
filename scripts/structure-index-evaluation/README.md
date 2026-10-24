# 结构索引阶段 0 实验

这是独立验证入口，直接引用 `src-tauri/src/text_document/structure_index/` 的共享解析适配器。已选定 pulldown-cmark 0.13.5 并开始原生服务接入；正文窗口迁移另行验收。

## 运行

从仓库根目录使用 PowerShell 7：

```powershell
pwsh -NoProfile -File scripts/structure-index-evaluation/run.ps1
cargo test --locked --manifest-path scripts/structure-index-evaluation/Cargo.toml
```

默认产物位于 `.artifacts/structure-index-20261024/`，可用 `-OutputDirectory` 指定新目录。首次需要下载规范和 Cargo 依赖。已有语料会复用，实际 SHA-256 记录在产物中；要重新取语料请使用新目录。

实验使用 Vitest/jsdom 读取当前实现作为 oracle，不启动浏览器或应用，不改变项目仅桌面、`pnpm run tauri dev` 的开发约定。

`run.ps1` 成功只表示实验执行完成。**是否可以选择内核取决于 `gate.json` 和 `REPORT.md`，不能用进程退出 0 代替兼容验收。** 门槛由规范、Nomo 对照、原生回归、引用、取消和收敛探针共同计算；不宣称整个应用迁移已经完成。

## 内容与边界

- CommonMark 0.31.2 全部 652 例；GFM 0.29 中显式选择的五类扩展 24 例。
- 49 份仓库语法样例；统一输出块范围／类型、标题文本／源码位置、有效定义。
- 读取边界 1/3/17/16384 字节；前部编辑后的范围收敛探针。
- 全局引用优先级的反例及权威事件适配原型。
- 32 KiB 片段的三轮解析／渲染时间、allocator 分配峰值；取消在片段返回后生效。

`structure.rs` 使用共享 Nomo 适配和有界扫描器验证候选。共享 Rust 回归另行覆盖超大围栏、混合 HTML 的保守原文边界、CRLF/UTF-8、标题转义、未解析依赖和 slug。应用会话、分页、增量身份及生命周期由 `src-tauri` 中的测试验证，不能用实验探针代替。

`compare-html.mjs` 保留每项原始结果，对有限的序列化差异作 DOM 等价分类；剩余差异仍需人工判读，不能全部叫作语法错误。该比较器包含防止忽略强调、文本空格和代码换行的自检。

`compare-structure.mjs` 比较当前真实输出，不将旧实现自动当作 CommonMark/GFM 规范。缺失 Nomo 适配不能证明候选库不适用，也不能标为兼容通过。

两候选的版本固定在本目录的 Cargo.toml/Cargo.lock。不要把两个候选同时加入应用依赖；只有完整门槛通过后，选定实现才能接入正式 Rust 服务。
