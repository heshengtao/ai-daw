/**
 * 浏览器端下载服务端生成的文件。
 *
 * 桌面端导出直接写本地磁盘；浏览器拿不到路径，只能由服务端把文件放进数据目录，
 * 这里用 `GET /download` 把它当成附件拉下来。
 */
export async function downloadFromServer(path: string): Promise<void> {
  if (!path) return;
  const name = path.split("/").pop() ?? "download";
  const link = document.createElement("a");
  link.href = `/download?path=${encodeURIComponent(path)}`;
  link.download = name;
  link.rel = "noopener";
  document.body.appendChild(link);
  link.click();
  link.remove();
}
