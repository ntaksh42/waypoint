// `tabs` API はタイトル・URLを読み、指定タブを前面化するためだけに使う。
// 取得した一覧は Native Messaging host 経由でローカルの waypoint だけへ送る。

const HOST_NAME = "com.ntaksh42.waypoint.tabs";
const browser = navigator.userAgent.includes("Edg/") ? "edge" : "chrome";
let port;
let snapshotTimer;

function scheduleSnapshot() {
  clearTimeout(snapshotTimer);
  snapshotTimer = setTimeout(sendSnapshot, 120);
}

async function sendSnapshot() {
  if (!port) return;
  const tabs = await chrome.tabs.query({});
  port.postMessage({
    type: "tabs",
    browser,
    tabs: tabs
      .filter((tab) => Number.isInteger(tab.id) && Number.isInteger(tab.windowId))
      .map((tab) => ({
        id: tab.id,
        windowId: tab.windowId,
        title: tab.title || "",
        url: tab.url || tab.pendingUrl || "",
      })),
  });
}

async function focusTab(message) {
  if (message.browser !== browser) return;
  try {
    const tab = await chrome.tabs.get(message.tabId);
    if (tab.windowId !== message.windowId) return;
    await chrome.windows.update(message.windowId, { focused: true });
    await chrome.tabs.update(message.tabId, { active: true });
  } catch {
    // タブが閉じた直後の要求は無視し、最新スナップショットで常駐側を追随させる。
    scheduleSnapshot();
  }
}

function connect() {
  try {
    port = chrome.runtime.connectNative(HOST_NAME);
  } catch {
    port = undefined;
    setTimeout(connect, 1000);
    return;
  }
  port.onMessage.addListener((message) => {
    if (message.type === "focus") focusTab(message);
    if (message.type === "ack" && !message.connected) setTimeout(sendSnapshot, 1000);
  });
  port.onDisconnect.addListener(() => {
    port = undefined;
    setTimeout(connect, 1000);
  });
  scheduleSnapshot();
}

chrome.tabs.onCreated.addListener(scheduleSnapshot);
chrome.tabs.onRemoved.addListener(scheduleSnapshot);
chrome.tabs.onUpdated.addListener(scheduleSnapshot);
chrome.tabs.onMoved.addListener(scheduleSnapshot);
chrome.tabs.onAttached.addListener(scheduleSnapshot);
chrome.tabs.onDetached.addListener(scheduleSnapshot);
chrome.tabs.onActivated.addListener(scheduleSnapshot);
chrome.windows.onRemoved.addListener(scheduleSnapshot);

connect();
