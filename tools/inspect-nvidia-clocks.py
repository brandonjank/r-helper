"""Read-only NVIDIA public NVAPI clock-offset capability probe.
API layouts and IDs: NVIDIA/nvapi nvapi.h and nvapi_interface.h (2026-09-02).
Reported driver bounds are not a stability guarantee. No setters are resolved.
"""
import ctypes as c, json, os, pathlib, runpy, sys
u=c.c_uint32;i=c.c_int32
class Delta(c.Structure):_fields_=[('value',i),('minimum',i),('maximum',i)]
class Clock(c.Structure):_fields_=[('domain',u),('kind',u),('flags',u),('delta',Delta),('data',u*5)]
class Voltage(c.Structure):_fields_=[('domain',u),('flags',u),('voltage',u),('delta',Delta)]
class State(c.Structure):_fields_=[('id',u),('flags',u),('clocks',Clock*8),('voltages',Voltage*4)]
class Overvoltage(c.Structure):_fields_=[('count',u),('voltages',Voltage*4)]
class Info(c.Structure):_fields_=[('version',u),('flags',u),('states_count',u),('clocks_count',u),('voltages_count',u),('states',State*16),('overvoltage',Overvoltage)]
def main():
    path=pathlib.Path(os.environ['SystemRoot'])/'System32/nvapi64.dll'
    trust=runpy.run_path(str(pathlib.Path(__file__).with_name('inspect-razer-native.py')))['verify_signature'];trust(path)
    dll=c.WinDLL(str(path),winmode=0x800);query=dll.nvapi_QueryInterface;query.argtypes=[u];query.restype=c.c_void_p
    def function(identifier,args):
        address=query(identifier)
        if not address:raise RuntimeError('NVAPI interface unavailable: '+hex(identifier))
        return c.CFUNCTYPE(i,*args)(address)
    init=function(0x0150e828,[]);unload=function(0xd22bdd7e,[])
    enum=function(0xe5ac921f,[c.POINTER(c.c_void_p),c.POINTER(u)])
    get=function(0x6ff81213,[c.c_void_p,c.POINTER(Info)])
    status=init()
    if status:raise RuntimeError('NVAPI initialization: '+str(status))
    try:
        handles=(c.c_void_p*64)();count=u();status=enum(handles,c.byref(count))
        if status or count.value>64:raise RuntimeError('NVAPI enumeration failed')
        result=[]
        for index in range(count.value):
            info=Info();info.version=c.sizeof(Info)|(3<<16);status=get(handles[index],c.byref(info))
            if status:result.append({'index':index,'status':status});continue
            if info.states_count>16 or info.clocks_count>8 or info.voltages_count>4:raise RuntimeError('NVAPI returned invalid counts')
            states=[]
            for state in info.states[:info.states_count]:
                clocks=[{'domain':clock.domain,'editable':bool(clock.flags&1),'offset_khz':clock.delta.value,'driver_min_khz':clock.delta.minimum,'driver_max_khz':clock.delta.maximum} for clock in state.clocks[:info.clocks_count]]
                states.append({'pstate':state.id,'editable':bool(state.flags&1),'clocks':clocks})
            result.append({'index':index,'editable':bool(info.flags&1),'states':states})
        with open(sys.argv[1],'x',encoding='utf-8') as output:json.dump({'read_only':True,'struct_size':c.sizeof(Info),'gpus':result,'note':'Driver bounds are not validated stable offsets.'},output,indent=2)
        print(json.dumps({'gpus':len(result),'struct_size':c.sizeof(Info)}))
    finally:unload()
if __name__=='__main__':main()
