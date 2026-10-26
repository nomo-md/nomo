// 普通构建的空模块；测试接口实现仅由显式测试构建打包。
export function startAutomationBridge(_app: unknown): () => void {
  return () => {};
}
