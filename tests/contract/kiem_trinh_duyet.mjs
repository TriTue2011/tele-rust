// Trình duyệt thật với API giả cục bộ; không gọi Telegram hoặc tải mã ngoài.
import assert from 'node:assert/strict';
import {readFileSync, mkdtempSync, rmSync} from 'node:fs';
import http from 'node:http';
import {spawn} from 'node:child_process';
import {once,EventEmitter} from 'node:events';

const phones=['+84900000001','+84900000002'];
const requests=[], errors=[], callActions=[], startedCalls=[], mediaActions=[], settingsPosts=[];
const csp=readFileSync('src/web.rs','utf8').match(/header::CONTENT_SECURITY_POLICY,"([^"]+)"/)[1];
const chatTitle='Hội thoại thử <em>A</em>';
let feed=[], calls=[], snapshots=0, snapshotStatus=409, rejectMedia=false;
let settings={camera:'black',mic:'rtsp://viewer:dummy-password@camera.example/sound',mic_format:'auto',speaker:'http://192.0.2.10/api/r1_speaker/intercom/DUMMY?k=dummy-key',playback:'file'};
const server=http.createServer(async (req,res)=>{
  const url=new URL(req.url,'http://localhost');
  const reply=value=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify(value));};
  if (['/','/ui/app.html','/index.html','/ui/index.html'].includes(url.pathname)) {
    res.setHeader('Content-Type','text/html');res.setHeader('Content-Security-Policy',csp);
    res.end(readFileSync(url.pathname.endsWith('index.html')?'web/index.html':'web/app.html'));return;
  }
  if (url.pathname==='/favicon.ico') {res.writeHead(204).end();return;}
  if (url.pathname==='/health') {reply({status:'ok',version:'test',accounts_total:2,accounts_online:2,mock_mode:false});return;}
  if (req.headers.authorization!=='Bearer browser-test-token') {res.statusCode=401;reply({error:{code:'UNAUTHORIZED'}});return;}
  requests.push(url);
  if (url.pathname==='/api/accounts') {reply({tai_khoan:phones.map((phone,i)=>({phone,display_name:'Tài khoản thử '+(i+1),status:'online'}))});return;}
  if (url.pathname==='/api/me') {reply({id:1,name:'Tài khoản thử',phone:''});return;}
  if (url.pathname==='/api/chats') {reply({chats:[{id:42,title:chatTitle,kind:'private',user_id:7,unread:0}]});return;}
  if (url.pathname==='/api/contacts') {reply({contacts:[{id:99,name:'Liên hệ thử'}]});return;}
  if (url.pathname==='/api/chats/42/messages') {reply({messages:[]});return;}
  if (url.pathname==='/api/feed') {
    if (url.searchParams.has('wait')) await new Promise(resolve=>setTimeout(resolve,100));
    const since=Number(url.searchParams.get('since')||0);
    reply({items:feed.filter(item=>item.id>since),cursor:feed.length,reset:false});return;
  }
  if (url.pathname==='/api/live/calls') {reply({calls});return;}
  if (url.pathname==='/api/live/settings') {
    if (req.method==='GET') {reply(settings);return;}
    let raw='';for await (const chunk of req) raw+=chunk;
    const body=JSON.parse(raw);settingsPosts.push({account:url.searchParams.get('account'),body});
    settings={...settings,...body};reply({ok:true});return;
  }
  if (url.pathname.startsWith('/api/live/snapshot/')) {
    const call=calls.find(item=>url.pathname===`/api/live/snapshot/${item.id}`);
    snapshots++;
    res.statusCode=call?.state==='active'?snapshotStatus:404;
    if (res.statusCode!==200) {reply({error:{code:{409:'NO_VIDEO',404:'CALL_ENDED',429:'RATE_LIMITED'}[res.statusCode]}});return;}
    res.setHeader('Content-Type','image/jpeg');res.end(Buffer.from('/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAABAAEDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDxyiiiuo5z/9k=','base64'));return;
  }
  if (req.method==='POST' && ['/api/live/accept','/api/live/hangup'].includes(url.pathname)) {
    let raw='';for await (const chunk of req) raw+=chunk;
    const body=JSON.parse(raw||'{}');callActions.push({path:url.pathname,id:body.id});
    const call=calls.find(item=>String(item.id)===String(body.id));
    if (call && url.pathname.endsWith('/hangup')) call.state='ended';
    reply(call||{id:body.id,state:'ended'});return;
  }
  if (req.method==='POST' && url.pathname==='/api/live/call') {
    let raw='';for await (const chunk of req) raw+=chunk;
    const body=JSON.parse(raw||'{}');startedCalls.push({url,id:body.user_id,video:body.video});
    const call={id:'fake-call-out',user_id:body.user_id,outgoing:true,video:!!body.video,state:'dialing',seconds:0,emojis:[],view:{frames:0,jpeg:0,size:''}};
    calls=[call];reply(call);return;
  }
  if (req.method==='POST' && ['/api/live/media','/api/live/playback'].includes(url.pathname)) {
    let raw='';for await (const chunk of req) raw+=chunk;
    const body=JSON.parse(raw);
    mediaActions.push({path:url.pathname,body});
    if (rejectMedia) {res.statusCode=502;reply({error:{code:'MEDIA_FAILED',message:'Thiết bị thử không phản hồi'}});return;}
    const call=calls.find(item=>item.id===body.id);
    if (body.mic_muted!==undefined) call.mic_muted=body.mic_muted;
    if (body.camera_off!==undefined) call.cam_off=body.camera_off;
    if (body.kind) call.playback.kind=body.kind;
    reply(call);return;
  }
  res.statusCode=404;reply({error:{code:'UNEXPECTED_PATH'}});
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const base=`http://127.0.0.1:${server.address().port}`;
const profile=mkdtempSync('.h2-browser-');
const chrome=spawn('google-chrome',['--headless=new','--no-sandbox','--disable-gpu','--no-first-run','--disable-background-networking','--disable-breakpad',`--user-data-dir=${profile}`,'--remote-debugging-pipe','about:blank'],{stdio:['ignore','ignore','pipe','pipe','pipe'],detached:true});
const pending=new Map(), navigation=new EventEmitter();let sequence=0,buffer='';
chrome.once('exit',(code,signal)=>{for(const handler of pending.values())handler.reject(new Error(`Chrome đã dừng (mã ${code}, tín hiệu ${signal})`));pending.clear();});
chrome.once('error',error=>{for(const handler of pending.values())handler.reject(error);pending.clear();});
chrome.stderr.on('data',chunk=>{if(process.env.CHROME_DEBUG)console.error('Chrome:',chunk.toString().trim());});
chrome.stdio[4].on('data',chunk=>{
  buffer+=chunk.toString();
  for (let end;(end=buffer.indexOf('\0'))>=0;) {
    const message=JSON.parse(buffer.slice(0,end));buffer=buffer.slice(end+1);
    if (message.id) {const handler=pending.get(message.id);pending.delete(message.id);if(handler)message.error?handler.reject(new Error(message.error.message)):handler.resolve(message.result);}
    if (message.method==='Runtime.exceptionThrown') errors.push(message.params.exceptionDetails.text);
    if (message.method==='Page.loadEventFired') navigation.emit(message.sessionId);
  }
});
function cdp(method,params={},sessionId) {
  return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject,method,detail:params.expression});chrome.stdio[3].write(JSON.stringify({id,method,params,sessionId})+'\0');});
}
const timer=setTimeout(()=>{console.error('Hết thời gian kiểm trình duyệt; Chrome DevTools đang chờ:',Array.from(pending.values()).map(x=>x.method+(x.detail?' ['+x.detail.slice(0,140)+']':'')).join(', ')||'không có lệnh chờ');process.exitCode=1;try{process.kill(-chrome.pid,'SIGKILL');}catch(e){}server.closeAllConnections();server.close();},40000);
try {
  const {targetId}=await cdp('Target.createTarget',{url:'about:blank'});
  const {sessionId}=await cdp('Target.attachToTarget',{targetId,flatten:true});
  const call=(method,params)=>cdp(method,params,sessionId);
  const evaluate=async expression=>{
    const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});
    assert(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value;
  };
  const wait=async condition=>{for(let n=0;n<150;n++){if(await evaluate(`Boolean(${condition})`))return true;await new Promise(resolve=>setTimeout(resolve,40));}throw new Error('Điều kiện không đạt: '+condition);};
  const waitServer=async condition=>{for(let n=0;n<150;n++){if(condition())return true;await new Promise(resolve=>setTimeout(resolve,40));}throw new Error('Máy chủ giả không nhận được yêu cầu dự kiến');};
  await call('Runtime.enable');await call('Page.enable');await call('Network.enable');
  await call('Network.setBlockedURLs',{urls:['https://*','http://cdnjs.cloudflare.com/*']});
  await call('Page.addScriptToEvaluateOnNewDocument',{source:`
    sessionStorage.setItem('gw_token','browser-test-token');
    window.__revokedBlobs=[];
    const revoke=URL.revokeObjectURL.bind(URL);
    URL.revokeObjectURL=(url)=>{window.__revokedBlobs.push(url);revoke(url);};
  `});
  await call('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});
  let loaded=once(navigation,sessionId);
  await call('Page.navigate',{url:base+'/'});await loaded;
  await wait("document.querySelectorAll('#account option').length===2 && document.querySelector('.chat')");
  assert.equal(await evaluate("document.querySelector('#account').value"),phones[0]);
  loaded=once(navigation,sessionId);
  await evaluate(`setTimeout(()=>{document.querySelector('#account').value=${JSON.stringify(phones[1])};document.querySelector('#account').dispatchEvent(new Event('change'));},0);`);
  await loaded;
  await wait(`document.querySelector('#account') && document.querySelector('#account').value===${JSON.stringify(phones[1])} && document.querySelector('.chat')`);
  assert.equal(await evaluate("getComputedStyle(document.querySelector('#call-overlay')).display"),'none');
  await waitServer(()=>requests.some(url=>url.pathname==='/api/live/calls' && url.searchParams.get('account')===phones[1]));
  const incoming={id:'fake-call-1',user_id:7,outgoing:false,video:true,state:'ringing',emojis:['🦊','🔒','<em>B</em>'],seconds:12,view:{frames:5,size:'640x360'},mic_muted:false,cam_off:false,playback:{kind:'push',speaker_configured:true}};
  calls=[incoming];
  // Không phát sự kiện cuộc gọi qua feed. Popup phải hiện trên danh sách nhờ polling.
  await wait("!document.querySelector('#call-overlay').hidden && document.querySelector('#call-name').textContent.includes('Hội thoại thử')");
  assert.equal(await evaluate("document.querySelector('#call-name').textContent"),'🎥 '+chatTitle);
  assert.equal(await evaluate("document.querySelector('#call-emojis').textContent"),'🦊 🔒 <em>B</em>');
  assert.equal(await evaluate("document.querySelector('#call-window em')"),null);
  assert.match(await evaluate("document.querySelector('#call-time').textContent"),/^00:1[2-9]$/);
  await evaluate("document.querySelector('#call-ring').click();document.querySelector('#call-ring').click()");
  assert.equal(await evaluate("document.querySelector('#call-ring').getAttribute('aria-pressed')"),'false');
  await evaluate("document.querySelector('#call-answer').click()");
  await wait("document.querySelector('#call-state').textContent==='Đang kết nối…' && document.querySelector('#call-answer').hidden");
  assert.deepEqual(callActions[0],{path:'/api/live/accept',id:incoming.id});
  incoming.state='active';
  await waitServer(()=>snapshots>0);
  assert.equal(await evaluate("!document.querySelector('#call-wait').hidden && document.querySelector('#call-image').hidden"),true);
  snapshotStatus=200;
  await wait("document.querySelector('#call-image').naturalWidth>0");
  const firstBlob=await evaluate("document.querySelector('#call-image').src");
  assert(firstBlob.startsWith('blob:'));
  await wait(`document.querySelector('#call-image').src!==${JSON.stringify(firstBlob)} && window.__revokedBlobs.includes(${JSON.stringify(firstBlob)})`);
  assert.match(await evaluate("document.querySelector('#call-meta').textContent"),/5 khung · 640x360/);
  assert.equal(await evaluate("document.querySelector('#call-window').getBoundingClientRect().width===920 && document.querySelector('#call-stage').clientHeight>0"),true);
  for (const [id,path,changes] of [
    ['mic','media',[{mic_muted:true},{mic_muted:false}]],
    ['camera','media',[{camera_off:true},{camera_off:false}]],
    ['speaker','playback',[{kind:'external'},{kind:'push'}]],
  ]) {
    for (const [index,body] of changes.entries()) {
      await evaluate(`document.querySelector('#call-${id}').click()`);
      await wait(`document.querySelector('#call-${id}').getAttribute('aria-pressed')==='${index===1}' && !document.querySelector('#call-${id}').disabled`);
      assert.deepEqual(mediaActions.at(-1),{path:'/api/live/'+path,body:{id:incoming.id,...body}});
    }
  }
  rejectMedia=true;
  await evaluate("document.querySelector('#call-mic').click()");
  await wait("document.querySelector('#call-error').textContent.includes('Thiết bị thử không phản hồi')");
  assert.equal(await evaluate("document.querySelector('#call-mic').getAttribute('aria-pressed')"),'true');
  rejectMedia=false;
  await call('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
  assert.equal(await evaluate("document.querySelector('#call-window').getBoundingClientRect().width===390 && document.querySelector('#call-stage').clientHeight>0 && document.documentElement.scrollWidth<=window.innerWidth"),true);
  const hiddenBlob=await evaluate(`(() => {
    const url=document.querySelector('#call-image').src;
    Object.defineProperty(document,'hidden',{configurable:true,value:true});
    document.dispatchEvent(new Event('visibilitychange'));
    return url;
  })()`);
  assert.equal(await evaluate(`window.__revokedBlobs.includes(${JSON.stringify(hiddenBlob)})`),true);
  const pausedCount=snapshots;
  await new Promise(resolve=>setTimeout(resolve,600));
  assert.equal(snapshots,pausedCount);
  await evaluate("delete document.hidden;document.dispatchEvent(new Event('visibilitychange'))");
  await wait("document.querySelector('#call-image').naturalWidth>0");
  snapshotStatus=429;
  await wait("document.querySelector('#call-wait').textContent.includes('giới hạn tải hình')");
  const throttledCount=snapshots;
  await new Promise(resolve=>setTimeout(resolve,500));
  assert.equal(snapshots,throttledCount);
  const callPollCount=()=>requests.filter(url=>url.pathname==='/api/live/calls').length;
  const before404=callPollCount();
  snapshotStatus=404;
  await wait("document.querySelector('#call-overlay').hidden");
  // Danh sách vẫn có cuộc gọi cũ, mô phỏng một lần đọc chậm hơn snapshot.
  await waitServer(()=>callPollCount()>before404);
  assert.equal(await evaluate("document.querySelector('#call-overlay').hidden"),true);
  await evaluate("document.querySelector('#call-settings-open').click()");
  await wait("document.querySelector('#call-settings').open");
  assert.equal(await evaluate("document.querySelector('#setting-camera').value"),'black');
  assert.equal(await evaluate("document.querySelector('#setting-mic').value+document.querySelector('#setting-speaker').value"),'');
  assert.equal(await evaluate("document.querySelector('#call-settings').textContent.includes('dummy-')"),false);
  assert.equal(await evaluate("document.querySelector('#call-settings').getBoundingClientRect().width<390"),true);
  await evaluate("document.querySelector('#setting-camera').value='file://camera.example/stream';document.querySelector('#call-settings-form').requestSubmit()");
  await wait("!document.querySelector('#call-settings-error').hidden");
  assert.equal(settingsPosts.length,0);
  await evaluate(`
    document.querySelector('#setting-camera').value='rtsp://camera.example/stream';
    document.querySelector('#setting-mic-format').value='s16le16k';
    document.querySelector('#setting-playback').value='external';
    document.querySelector('#call-settings-form').requestSubmit();
  `);
  await wait("!document.querySelector('#call-settings').open");
  assert.deepEqual(settingsPosts[0],{account:phones[1],body:{camera:'rtsp://camera.example/stream',mic_format:'s16le16k',playback:'external'}});
  await evaluate("document.querySelector('#call-settings-open').click()");
  await wait("document.querySelector('#call-settings').open && document.querySelector('#setting-camera').value==='rtsp://camera.example/stream'");
  await evaluate(`
    document.querySelector('#setting-mic').value='http://192.0.2.20/mic';
    document.querySelector('#setting-speaker-clear').click();
    document.querySelector('#call-settings-form').requestSubmit();
  `);
  await wait("!document.querySelector('#call-settings').open");
  assert.deepEqual(settingsPosts[1],{account:phones[1],body:{mic:'http://192.0.2.20/mic',speaker:''}});
  await evaluate("document.querySelector('#call-settings-open').click()");
  await wait("document.querySelector('#call-settings').open");
  calls=[{id:'fake-call-unknown',user_id:123456,outgoing:false,video:false,state:'ringing',seconds:0}];
  await wait("!document.querySelector('#call-overlay').hidden && !document.querySelector('#call-settings').open");
  assert.equal(await evaluate("document.querySelector('#call-name').textContent"),'📞 Người dùng · •••456');
  calls[0].state='ended';
  await wait("document.querySelector('#call-overlay').hidden");
  await evaluate("document.querySelector('#t-contacts').click()");
  await wait("document.querySelector('#list').textContent.includes('Liên hệ thử')");
  calls=[{id:'fake-call-2',user_id:99,outgoing:false,video:false,state:'ringing',seconds:0}];
  await wait("!document.querySelector('#call-overlay').hidden && document.querySelector('#call-name').textContent.includes('Liên hệ thử')");
  assert.equal(await evaluate("document.querySelector('#call-end').textContent"),'Từ chối');
  await evaluate("document.querySelector('#call-end').click()");
  await wait("document.querySelector('#call-overlay').hidden");
  assert.deepEqual(callActions[1],{path:'/api/live/hangup',id:'fake-call-2'});
  await call('Emulation.setDeviceMetricsOverride',{width:1280,height:900,deviceScaleFactor:1,mobile:false});
  await evaluate("document.querySelector('#t-chats').click()");
  await evaluate("document.querySelector('.chat').click()");
  await wait("document.querySelector('#composer').hidden===false");
  feed=[{id:1,type:'message',account:phones[0],chat_id:42,message:{id:51,chat_id:42,text:'KHONG_DUOC_LAN',kind:'text',date:1,outgoing:true}},{id:2,type:'message',account:phones[1],chat_id:42,message:{id:52,chat_id:42,text:'TIN_DUNG_TAI_KHOAN',kind:'text',date:1,outgoing:true}}];
  await wait("document.querySelector('#msgs').textContent.includes('TIN_DUNG_TAI_KHOAN')");
  assert.equal(await evaluate("document.querySelector('#msgs').textContent.includes('KHONG_DUOC_LAN')"),false);
  assert(requests.some(url=>url.pathname==='/api/chats/42/messages' && url.searchParams.get('account')===phones[1]));
  await evaluate("document.querySelector('#call-a').click()");
  await wait("document.querySelector('#call-overlay').hidden===false && document.querySelector('#call-name').textContent.includes('Hội thoại thử')");
  assert.deepEqual(startedCalls[0].id,7);assert.equal(startedCalls[0].video,false);
  await evaluate("document.querySelector('#call-controls button.red').click()");
  await waitServer(()=>callActions.some(a=>a.path==='/api/live/hangup' && a.id==='fake-call-out'));
  await wait("document.querySelector('#call-overlay').hidden===true");
  assert.equal(readFileSync('web/app.html','utf8').includes('innerHTML'),false);
  assert.equal(await evaluate("document.documentElement.scrollWidth<=window.innerWidth"),true);
  await call('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
  assert.equal(await evaluate("document.documentElement.scrollWidth<=window.innerWidth"),true);
  loaded=once(navigation,sessionId);
  await call('Page.navigate',{url:base+'/ui/index.html'});await loaded;
  await wait("document.querySelectorAll('.acc').length===2");
  assert.equal(await evaluate("document.querySelector('#login').hidden"),false);
  assert.deepEqual(errors,[]);
  console.log(`Chrome: gọi đến qua polling ở Hội thoại/Danh bạ, gọi đi, Trả lời/Từ chối/Cúp máy, emoji dạng văn bản, JPEG 409/200/429/404, thu hồi blob, 6 thao tác mic/camera/loa và lỗi 502, cài đặt đủ 5 trường và che thông tin xác thực, 390px và 1280px. ${errors.length} lỗi JavaScript.`);
} finally {
  clearTimeout(timer);if(chrome.exitCode===null&&chrome.signalCode===null){const exited=once(chrome,'exit');try{process.kill(-chrome.pid,'SIGTERM');}catch(e){chrome.kill('SIGTERM');}await exited;}server.closeAllConnections();server.close();rmSync(profile,{recursive:true,force:true});
}
