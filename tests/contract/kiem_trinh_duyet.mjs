// Trình duyệt thật với API giả cục bộ; không gọi Telegram hoặc tải mã ngoài.
import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdtempSync, rmSync} from 'node:fs';
import http from 'node:http';
import {spawn} from 'node:child_process';
import {once,EventEmitter} from 'node:events';

const phones=['+84900000001','+84900000002'];
const requests=[], errors=[];
let feed=[];
const server=http.createServer(async (req,res)=>{
  const url=new URL(req.url,'http://localhost');
  const reply=value=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify(value));};
  if (url.pathname==='/' || url.pathname==='/ui/app.html') {res.setHeader('Content-Type','text/html');res.end(readFileSync('web/app.html'));return;}
  if (url.pathname==='/index.html' || url.pathname==='/ui/index.html') {res.setHeader('Content-Type','text/html');res.end(readFileSync('web/index.html'));return;}
  if (url.pathname==='/favicon.ico') {res.writeHead(204).end();return;}
  if (url.pathname==='/health') {reply({status:'ok',version:'test',accounts_total:2,accounts_online:2,mock_mode:false});return;}
  if (req.headers.authorization!=='Bearer browser-test-token') {res.statusCode=401;reply({error:{code:'UNAUTHORIZED'}});return;}
  requests.push(url);
  if (url.pathname==='/api/accounts') {reply({tai_khoan:phones.map((phone,i)=>({phone,display_name:'Tài khoản thử '+(i+1),status:'online'}))});return;}
  if (url.pathname==='/api/me') {reply({id:1,name:'Tài khoản thử',phone:''});return;}
  if (url.pathname==='/api/chats') {reply({chats:[{id:42,title:'Hội thoại thử',kind:'private',user_id:7,unread:0}]});return;}
  if (url.pathname==='/api/chats/42/messages') {reply({messages:[]});return;}
  if (url.pathname==='/api/feed') {
    if (url.searchParams.has('wait')) await new Promise(resolve=>setTimeout(resolve,100));
    const since=Number(url.searchParams.get('since')||0);
    reply({items:feed.filter(item=>item.id>since),cursor:feed.length,reset:false});return;
  }
  reply({ok:true,calls:[]});
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const base=`http://127.0.0.1:${server.address().port}`;
const profile=mkdtempSync('.h2-browser-');
const chrome=spawn('google-chrome',['--headless=new','--no-sandbox','--disable-gpu','--no-first-run','--disable-background-networking','--disable-breakpad',`--user-data-dir=${profile}`,'--remote-debugging-pipe','about:blank'],{stdio:['ignore','ignore','ignore','pipe','pipe']});
const pending=new Map(), navigation=new EventEmitter();let sequence=0,buffer='';
chrome.stdio[4].on('data',chunk=>{
  buffer+=chunk.toString();
  for (let end;(end=buffer.indexOf('\0'))>=0;) {
    const message=JSON.parse(buffer.slice(0,end));buffer=buffer.slice(end+1);
    if (message.id) {const handler=pending.get(message.id);pending.delete(message.id);message.error?handler.reject(new Error(message.error.message)):handler.resolve(message.result);}
    if (message.method==='Runtime.exceptionThrown') errors.push(message.params.exceptionDetails.text);
    if (message.method==='Page.loadEventFired') navigation.emit(message.sessionId);
  }
});
function cdp(method,params={},sessionId) {
  return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});chrome.stdio[3].write(JSON.stringify({id,method,params,sessionId})+'\0');});
}
let timer=setTimeout(()=>{console.error('Hết thời gian kiểm trình duyệt');chrome.kill('SIGKILL');process.exitCode=1;},25000);
try {
  const {targetId}=await cdp('Target.createTarget',{url:'about:blank'});
  const {sessionId}=await cdp('Target.attachToTarget',{targetId,flatten:true});
  const call=(method,params)=>cdp(method,params,sessionId);
  const evaluate=async expression=>{
    const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});
    assert(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value;
  };
  const wait=condition=>evaluate(`new Promise((resolve,reject)=>{let n=0;const timer=setInterval(()=>{if (${condition}) {clearInterval(timer);resolve(true);}else if (++n>150) {clearInterval(timer);reject(new Error('Điều kiện không đạt'));}},40);})`);
  await call('Runtime.enable');await call('Page.enable');await call('Network.enable');
  await call('Network.setBlockedURLs',{urls:['https://*','http://cdnjs.cloudflare.com/*']});
  await call('Page.addScriptToEvaluateOnNewDocument',{source:"sessionStorage.setItem('gw_token','browser-test-token');"});
  await call('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});
  let loaded=once(navigation,sessionId);
  await call('Page.navigate',{url:base+'/'});await loaded;
  await wait("document.querySelectorAll('#account option').length===2 && document.querySelector('.chat')");
  assert.equal(await evaluate("document.querySelector('#account').value"),phones[0]);
  loaded=once(navigation,sessionId);
  await evaluate(`setTimeout(()=>{document.querySelector('#account').value=${JSON.stringify(phones[1])};document.querySelector('#account').dispatchEvent(new Event('change'));},0);`);
  await loaded;
  await wait(`document.querySelector('#account') && document.querySelector('#account').value===${JSON.stringify(phones[1])} && document.querySelector('.chat')`);
  await evaluate("document.querySelector('.chat').click()");
  await wait("document.querySelector('#composer').hidden===false");
  feed=[{id:1,type:'message',account:phones[0],chat_id:42,message:{id:51,chat_id:42,text:'KHONG_DUOC_LAN',kind:'text',date:1,outgoing:true}},{id:2,type:'message',account:phones[1],chat_id:42,message:{id:52,chat_id:42,text:'TIN_DUNG_TAI_KHOAN',kind:'text',date:1,outgoing:true}}];
  await wait("document.querySelector('#msgs').textContent.includes('TIN_DUNG_TAI_KHOAN')");
  assert.equal(await evaluate("document.querySelector('#msgs').textContent.includes('KHONG_DUOC_LAN')"),false);
  assert(requests.some(url=>url.pathname==='/api/chats/42/messages' && url.searchParams.get('account')===phones[1]));
  const {data}=await call('Page.captureScreenshot',{format:'png'});writeFileSync('.h2-web.png',Buffer.from(data,'base64'));
  await call('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
  assert.equal(await evaluate("document.documentElement.scrollWidth<=window.innerWidth"),true);
  loaded=once(navigation,sessionId);
  await call('Page.navigate',{url:base+'/ui/index.html'});await loaded;
  await wait("document.querySelectorAll('.acc').length===2");
  assert.equal(await evaluate("document.querySelector('#login').hidden"),false);
  assert.deepEqual(errors,[]);
  console.log('Chrome: 2 tài khoản, chuyển tài khoản, định tuyến lịch sử, lọc tin khác tài khoản, trang đăng nhập và màn hình 390px đạt; 0 lỗi JavaScript.');
} finally {
  clearTimeout(timer);chrome.kill('SIGTERM');await once(chrome,'exit');server.closeAllConnections();server.close();rmSync(profile,{recursive:true,force:true});rmSync('.h2-web.png',{force:true});
}
