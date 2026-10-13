import { useEffect, useRef, useState } from 'react';
import { ExternalLink } from 'lucide-react';
import { startBrowserSignIn, pollBrowserSignIn, cancelBrowserSignIn, type BrowserSignIn as Request, type CloudStatus } from '../lib/cloud';
export function BrowserSignIn({onChange}: {onChange: (status:CloudStatus)=>void}) {
  const [request,setRequest] = useState<Request|null>(null), [busy,setBusy] = useState(false), [error,setError] = useState('');
  const mounted = useRef(true), changed = useRef(onChange);
  changed.current=onChange;
  useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;};},[]);
  useEffect(()=>{
    if(!request) return;
    let stopped=false;
    let timer:ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        if(Date.now()>=Date.parse(request!.expiresAt)) throw new Error('Sign-in expired. Please start again.');
        const status = await pollBrowserSignIn(request!.flowId);
        if(stopped) return;
        if(status) {setRequest(null);changed.current(status);return;}
        timer=setTimeout(()=>void poll(),2500);
      } catch(reason) {
        if(stopped) return;
        console.error('[Browser Sign-In] CHECK_FAILED');
        setError(typeof reason==='string'?reason:reason instanceof Error?reason.message:'Could not check sign-in. Please try again.');
        setRequest(null);
      }
    }
    timer=setTimeout(()=>void poll(),2500);
    return()=>{stopped=true;clearTimeout(timer);void cancelBrowserSignIn(request.flowId).catch(()=>console.error('[Browser Sign-In] CANCEL_FAILED'));};
  },[request]);
  async function start() {
    setBusy(true);setError('');
    try {
      const next=await startBrowserSignIn();
      if(!mounted.current) {await cancelBrowserSignIn(next.flowId);return;}
      setRequest(next);
    } catch(reason) {if(mounted.current) {console.error('[Browser Sign-In] START_FAILED');setError(typeof reason==='string'?reason:'Could not open sign-in. Please try again.');}}
    finally {if(mounted.current)setBusy(false);}
  }
  return <>
    <p>Sign in securely in your browser with Google, Apple or email. Local tracking works without an account.</p>
    {request ? <>
      <div className="cloud-status" role="status">Waiting for Browser Sign-In</div>
      <p>Enter this connection code on the DevClash page:</p>
      <p className="desktop-connection-code">{request.displayCode}</p>
      <p className="cloud-note">Only approve a request you started. This code expires in ten minutes. Return here after connecting.</p>
      <button className="button" onClick={()=>setRequest(null)}>Cancel Sign-In</button>
    </> : <button className="button primary" disabled={busy} onClick={()=>void start()}>{busy?'Opening Browser…':error?'Try Again':'Sign In With Browser'}<ExternalLink size={16}/></button>}
    <p className="cloud-note">Cloud uploads start only when you enable them. Public rankings need a separate opt-in.</p>
    {error && <p className="cloud-error" role="alert">{error}</p>}
  </>;
}
