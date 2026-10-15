"""Optional read-only research against installed, signed Razer display helpers.
No vendor files are distributed. Each getter runs in its own bounded child.
"""
import argparse, ctypes, json, os, pathlib, subprocess, sys
GETTERS=("GetDLLVersion","GetBladeLEDPanelProducerShortName","GetBladeScreenResolution","GetBladeFirmwareInfo","GetGPUValueStr")
def library():
    base=pathlib.Path(os.environ['LOCALAPPDATA'])/'Razer/RazerAppEngine/User Data/Apps/Common/bladeCommon'
    candidates=sorted(base.glob('blade2Native_v*.dll'))
    if not candidates:raise RuntimeError('Installed Razer display helper not found')
    return max(candidates,key=lambda path:tuple(map(int,path.stem.removeprefix('blade2Native_v').split('.'))))
def clean(value):
    if isinstance(value,dict):return {k:clean(v) for k,v in value.items() if not any(t in k.lower() for t in ('serial','machine','username','computername','container','instanceid','deviceid','path'))}
    if isinstance(value,list):return [clean(v) for v in value[:256]]
    return value

def verify_signature(path):
    from ctypes import wintypes as w
    class GUID(ctypes.Structure):
        _fields_=[('a',w.DWORD),('b',w.WORD),('c',w.WORD),('d',ctypes.c_ubyte*8)]
    class FILE(ctypes.Structure):
        _fields_=[('size',w.DWORD),('path',w.LPCWSTR),('handle',w.HANDLE),('subject',ctypes.c_void_p)]
    class DATA(ctypes.Structure):
        _fields_=[('size',w.DWORD),('policy',ctypes.c_void_p),('sip',ctypes.c_void_p),('ui',w.DWORD),('revocation',w.DWORD),('choice',w.DWORD),('file',ctypes.POINTER(FILE)),('action',w.DWORD),('state',w.HANDLE),('url',w.LPCWSTR),('flags',w.DWORD),('context',w.DWORD),('settings',ctypes.c_void_p)]
    action=GUID(0x00aac56b,0xcd44,0x11d0,(ctypes.c_ubyte*8)(0x8c,0xc2,0,0xc0,0x4f,0xc2,0x95,0xee))
    file=FILE(ctypes.sizeof(FILE),str(path),None,None)
    data=DATA();data.size=ctypes.sizeof(DATA);data.ui=2;data.choice=1;data.file=ctypes.pointer(file);data.action=1;data.flags=0x1000
    trust=ctypes.WinDLL('wintrust.dll',winmode=0x800).WinVerifyTrust
    trust.argtypes=[w.HWND,ctypes.POINTER(GUID),ctypes.POINTER(DATA)];trust.restype=w.LONG
    try:status=trust(None,ctypes.byref(action),ctypes.byref(data))
    finally:
        data.action=2;trust(None,ctypes.byref(action),ctypes.byref(data))
    if status!=0:raise RuntimeError('Native DLL signature validation failed: '+hex(status & 0xffffffff))
    return {'status':'Valid','verification':'Windows WinVerifyTrust; cached certificate validation'}

def child(name):
    if name not in GETTERS:raise RuntimeError('Getter is not permitted')
    path=library()
    verify_signature(path)
    # Restricted DLL search: its signed installation directory and System32.
    handle=ctypes.WinDLL(str(path),winmode=0x1100)
    function=getattr(handle,name);function.argtypes=[];function.restype=ctypes.c_void_p
    release=handle.FreeMalloc;release.argtypes=[ctypes.c_void_p];release.restype=None
    pointer=function()
    if not pointer:raise RuntimeError('Getter returned no data')
    try:
        data=ctypes.string_at(pointer)  # Documented NUL-terminated allocation; child isolates native faults.
    finally:release(pointer)
    if len(data)>=65536:raise RuntimeError('Getter response exceeds limit')
    result=data.decode('utf-8')
    try:result=json.loads(result)
    except json.JSONDecodeError:pass
    print(json.dumps({'getter':name,'value':clean(result)}))
def main():
    parser=argparse.ArgumentParser();parser.add_argument('--output',type=pathlib.Path);parser.add_argument('--getter',choices=GETTERS)
    args=parser.parse_args()
    if args.getter:return child(args.getter)
    if args.output is None:parser.error('--output is required')
    path=library()
    signature=verify_signature(path)
    results=[]
    for name in GETTERS:
        try:
            proc=subprocess.run([sys.executable,__file__,'--getter',name],capture_output=True,text=True,timeout=15)
            results.append(json.loads(proc.stdout) if proc.returncode==0 else {'getter':name,'error':'Isolated getter failed','exit_code':proc.returncode})
        except subprocess.TimeoutExpired:results.append({'getter':name,'error':'Getter timed out; child terminated'})
    with args.output.open('x',encoding='utf-8') as output:json.dump({'read_only':True,'helper':path.name,'signature':signature,'getters':results},output,indent=2)
    print(json.dumps({'output':str(args.output),'getters':len(results)}))
if __name__=='__main__':main()
