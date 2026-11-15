// Edge のお気に入り編集画面 (waypoint-settings.exe --bookmarks、FR-6.17) との橋渡し。
// お気に入りの読み書きはすべて chrome.bookmarks 経由で行うため、Edge の同期にも
// 通常の編集として乗る。Native Messaging host はパイプを中継するだけで中身を見ない。
// Chrome は対象外なので、Edge で動いているときだけ接続する。
(() => {
  const HOST_NAME = "com.ntaksh42.waypoint.bookmarks";
  // host が無い (waypoint 未起動で未登録) ときに起動のたびに失敗し続けないよう、間隔を空けて試す
  const RETRY_MS = 10000;
  const SNAPSHOT_DELAY_MS = 300;

  if (!navigator.userAgent.includes("Edg/")) return;

  let port;
  let snapshotTimer;

  function connect() {
    if (port) return;
    try {
      port = chrome.runtime.connectNative(HOST_NAME);
    } catch {
      port = undefined;
      setTimeout(connect, RETRY_MS);
      return;
    }
    port.onMessage.addListener(onMessage);
    port.onDisconnect.addListener(() => {
      port = undefined;
      setTimeout(connect, RETRY_MS);
    });
  }

  function slim(node) {
    const out = { id: node.id, title: node.title, parentId: node.parentId, index: node.index };
    if (node.url !== undefined) out.url = node.url;
    if (node.children) out.children = node.children.map(slim);
    return out;
  }

  async function sendSnapshot() {
    if (!port) return;
    const [root] = await chrome.bookmarks.getTree();
    port.postMessage({ type: "snapshot", tree: root.children.map(slim) });
  }

  function scheduleSnapshot() {
    clearTimeout(snapshotTimer);
    snapshotTimer = setTimeout(sendSnapshot, SNAPSHOT_DELAY_MS);
  }

  async function run(op, args) {
    switch (op) {
      case "update": {
        const changes = {};
        if (args.title !== undefined) changes.title = args.title;
        if (args.url !== undefined) changes.url = args.url;
        await chrome.bookmarks.update(args.id, changes);
        return {};
      }
      case "move": {
        const destination = { parentId: args.parentId };
        if (args.index !== undefined) destination.index = args.index;
        await chrome.bookmarks.move(args.id, destination);
        return {};
      }
      case "remove":
        if (args.tree) await chrome.bookmarks.removeTree(args.id);
        else await chrome.bookmarks.remove(args.id);
        return {};
      case "create": {
        const spec = { parentId: args.parentId, title: args.title };
        if (args.url !== undefined) spec.url = args.url;
        if (args.index !== undefined) spec.index = args.index;
        const created = await chrome.bookmarks.create(spec);
        return { id: created.id };
      }
      default:
        throw new Error("Unsupported operation: " + op);
    }
  }

  async function onMessage(message) {
    if (message.type === "requestSnapshot") {
      await sendSnapshot();
      return;
    }
    if (message.type !== "command") return;
    try {
      const data = await run(message.op, message.args || {});
      port?.postMessage({ type: "result", reqId: message.reqId, ok: true, ...data });
    } catch (error) {
      port?.postMessage({
        type: "result",
        reqId: message.reqId,
        ok: false,
        error: String((error && error.message) || error),
      });
    }
    scheduleSnapshot();
  }

  // 他の端末からの同期や Edge 上での変更も、開いている編集画面へ反映する
  for (const event of ["onCreated", "onRemoved", "onChanged", "onMoved", "onChildrenReordered", "onImportEnded"]) {
    chrome.bookmarks[event].addListener(scheduleSnapshot);
  }

  connect();
})();
