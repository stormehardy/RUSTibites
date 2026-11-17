import json,zipfile,sys
src='C:/Temp/bd/bd2.zip'; dst='C:/Temp/bd/starve.zip'
zi=zipfile.ZipFile(src); zo=zipfile.ZipFile(dst,'w',zipfile.ZIP_DEFLATED)
for n in zi.namelist():
    data=zi.read(n)
    if n.startswith('bibites/') and n.endswith('.bb8'):
        d=json.loads(data.decode('utf-8-sig'))
        for nd in d['brain']['Nodes']:
            i=int(nd['Index'])
            if i>=33:
                nd['Type']=2; v={33:0.3,34:0.2}.get(i,0.0)
                nd['baseActivation']=v; nd['Value']=v; nd['LastInput']=0.0; nd['LastOutput']=v
        d['brain']['Synapses']=[]
        d['body']['energy']=min(d['body']['energy'],25.0)
        data=json.dumps(d).encode('utf-8')
    zo.writestr(n,data)
zo.close(); print(dst)
