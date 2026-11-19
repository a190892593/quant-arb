// 把 dist 发布到后端静态目录 ../ui（后端 config/default.toml 的 ui_dir 指向它）。
// 只做覆盖复制，不删除目录：避免误删与不可恢复操作；旧哈希文件不再被 index.html 引用即可。
import { cp, mkdir } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const dist = resolve(here, "..", "dist");
const target = resolve(here, "..", "..", "ui");

await mkdir(target, { recursive: true });
await cp(dist, target, { recursive: true, force: true });
console.log(`published ${dist} → ${target}`);
