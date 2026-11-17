import sys
sys.path.insert(0, 'C:/Temp/bd')
import make_fixture as mf, json, os, glob, zipfile, copy

def zero_brain2(d, outs):
    for node in d['brain']['Nodes']:
        idx = int(node['Index'])
        if idx >= 33:
            node['Type'] = 2
            v = outs.get(idx, 0.0)
            node['baseActivation'] = v; node['Value'] = v; node['LastInput'] = 0.0; node['LastOutput'] = v
    d['brain']['Synapses'] = []
    d['body']['stomach']['content'] = []
    return d

orig = mf.zero_brain
def make(name, outs, pellet, material, gap):
    mf.zero_brain = lambda d, accel: zero_brain2(d, outs)
    z = mf.make(name, 0.0, pellet=pellet, gap=gap, material=material)
    mf.zero_brain = orig
    return z

if __name__ == '__main__':
    # 33 Accelerate, 38 Want2Eat, 47 Want2Attack
    print(make('eatmeat100', {33: 0.3, 38: 1.0}, 100, 'Meat', 30.0))
    print(make('eatmeat400', {33: 0.3, 38: 1.0}, 400, 'Meat', 30.0))
    print(make('bitemeat400', {33: 0.3, 38: 0.0, 47: 1.0}, 400, 'Meat', 30.0))
    print(make('bothmeat400', {33: 0.3, 38: 1.0, 47: 1.0}, 400, 'Meat', 30.0))
    print(make('eatplant100', {33: 0.3, 38: 1.0}, 100, 'Plant', 30.0))
