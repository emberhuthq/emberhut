import { Directory, File, Paths } from "expo-file-system";
import * as Sharing from "expo-sharing";
import { api } from "@/services";

// A server file leaves the app as itself: the whole original, under its real name, kept in the cache once it's here.

/** Its place in the cache: a folder per server path, so two files with the same name stay apart. */
function cached(path: string, name: string): File {
  const folder = new Directory(Paths.cache, "emberhut-files", path.replace(/[^A-Za-z0-9]/g, "-"));
  if (!folder.exists) folder.create({ intermediates: true });
  return new File(folder, name.replace(/\//g, "-"));
}

/** The file here, downloaded once with this device's session. */
async function download(path: string, name: string): Promise<File> {
  const file = cached(path, name);
  if (file.exists && file.size > 0) return file;
  const { uri, headers } = api.files.source(path);
  return File.downloadFileAsync(uri, file, { headers, idempotent: true });
}

/** Opens it in the app Android picks for it (a PDF reader, a spreadsheet app). */
export async function openOutside(path: string, name: string): Promise<void> {
  const file = await download(path, name);
  await file.preview({ title: name });
}

/** The share sheet: send it on, or save it to Drive or Files. */
export async function share(path: string, name: string): Promise<void> {
  const file = await download(path, name);
  await Sharing.shareAsync(file.uri, { dialogTitle: name, mimeType: file.type || undefined });
}

/** A server file's text (a report's page), through the session. */
export async function readText(path: string): Promise<string> {
  return new TextDecoder().decode(await api.files.bytes(path));
}
