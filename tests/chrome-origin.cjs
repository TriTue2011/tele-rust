// Nạp HTML vào Chrome headless dưới một ORIGIN HTTP thật (http://tele-rust.test/) thay vì about:blank + Page.setDocumentContent.
// Lý do (CI 22/11): trang about:blank có origin mờ ⇒ Chromium/Chrome bản mới/snap ném SecurityError khi đọc localStorage
// («Access is denied for this document»), còn ứng dụng thật luôn chạy trên HTTP. Yêu cầu HTML được CDP `Fetch.fulfillRequest` trả về,
// không cần mở cổng mạng. Dùng: await openHttpOrigin(cdp, ws, html);
const ORIGIN = 'http://tele-rust.test/';

async function openHttpOrigin(cdp, ws, html) {
  await cdp('Page.enable');
  await cdp('Fetch.enable', { patterns: [{ urlPattern: `${ORIGIN}*` }] });
  const body = Buffer.from(html, 'utf8').toString('base64');
  ws.addEventListener('message', ({ data }) => {
    const msg = JSON.parse(data);
    if (msg.method !== 'Fetch.requestPaused') return;
    const isPage = msg.params.request.url === ORIGIN;   // favicon và tài nguyên khác dưới origin giả: 404 rỗng
    cdp('Fetch.fulfillRequest', {
      requestId: msg.params.requestId,
      responseCode: isPage ? 200 : 404,
      responseHeaders: [{ name: 'Content-Type', value: 'text/html; charset=utf-8' }],
      body: isPage ? body : '',
    }).catch(() => {});
  });
  await cdp('Page.navigate', { url: ORIGIN });
  for (let n = 0; n < 200; n++) {
    const r = await cdp('Runtime.evaluate', { expression: 'location.href === ' + JSON.stringify(ORIGIN) + ' && document.readyState === "complete"', returnByValue: true });
    if (r.result.value === true) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error('Không nạp được trang dưới origin ' + ORIGIN);
}

module.exports = { openHttpOrigin, ORIGIN };
