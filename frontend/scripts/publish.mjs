// 把 dist 发布到后端静态目录 ../ui（后端 config/default.toml 的 ui_dir 指向它）。
//
// 流程：复制 → 按 index.html 引用清理旧哈希文件。
// 之前只覆盖复制，每次构建都会留下一个新的 index-<hash>.js，
// 50 次构建后 ui/assets 累积 50 个 bundle（58.9 MB），全部进了版本库。
// 现在只保留当前 index.html 引用到的资源，其余删除。
import { cp, mkdir, readdir, readFile, unlink } from "node:fs/promises";
import { resolve, dirname, basename } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const dist = resolve(here, "..", "dist");
const target = resolve(here, "..", "..", "ui");
const assetsDir = resolve(target, "assets");

await mkdir(target, { recursive: true });
await cp(dist, target, { recursive: true, force: true });

// index.html 里引用到的资源（js / css / 其他）
const html = await readFile(resolve(target, "index.html"), "utf8");
const referenced = new Set(
  [...html.matchAll(/(?:\.\/)?assets\/([^"'\s)>]+)/g)].map((m) => basename(m[1])),
);

let removed = 0;
let bytes = 0;
for (const name of await readdir(assetsDir)) {
  if (referenced.has(name)) continue;
  const path = resolve(assetsDir, name);
  try {
    const { size } = await import("node:fs/promises").then((fs) => fs.stat(path));
    await unlink(path);
    removed += 1;
    bytes += size;
  } catch {
    /* 目录或已删除，忽略 */
  }
}

const kept = (await readdir(assetsDir)).length;
console.log(
  `published ${dist} → ${target} · 保留 ${kept} 个资源，清理 ${removed} 个旧文件（${(bytes / 1024 / 1024).toFixed(1)} MB）`,
);
