import AppKit
let input=".cache/probes/drt-lift/run3/DSC04399-current-matched.tif"
let image=NSImage(contentsOfFile:input)!
let out=NSImage(size:NSSize(width:900,height:600))
out.lockFocus();image.draw(in:NSRect(x:0,y:0,width:900,height:600))
for (label,x,y) in [("A",3000,3000),("B",3050,3350),("C",3200,3700),("D",4100,2800),("E",5100,3250),("F",4750,3450),("G",4270,3400),("H",3120,3250)] {
    let sx=Double(x-192)/8136*900,sy=600-Double(y-96)/5424*600
    NSColor.systemYellow.setStroke();let box=NSBezierPath(rect:NSRect(x:sx,y:sy-7,width:7,height:7));box.lineWidth=2;box.stroke()
    (label as NSString).draw(at:NSPoint(x:sx+7,y:sy),withAttributes:[.font:NSFont.boldSystemFont(ofSize:18),.foregroundColor:NSColor.systemYellow])
}
out.unlockFocus();let rep=NSBitmapImageRep(data:out.tiffRepresentation!)!
try rep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:".cache/probes/film-model-images/patches.png"))
