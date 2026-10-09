import AppKit
let dir=".cache/probes/dmin-trace/run2/"
func sheet(_ rows:[[(String,String)]],_ width:Double,_ height:Double,_ name:String) throws {
    let size=NSSize(width:width*3,height:(height+26)*Double(rows.count))
    let out=NSImage(size:size);out.lockFocus();NSColor(calibratedWhite:0.12,alpha:1).setFill();NSRect(origin:.zero,size:size).fill()
    for(j,row) in rows.enumerated(){for(i,item) in row.enumerated(){
        let x=Double(i)*width,y=Double(rows.count-1-j)*(height+26)
        NSImage(contentsOfFile:dir+item.0+".tif")!.draw(in:NSRect(x:x,y:y+26,width:width,height:height))
        (item.1 as NSString).draw(at:NSPoint(x:x+6,y:y+5),withAttributes:[.font:NSFont.systemFont(ofSize:12),.foregroundColor:NSColor.white])
    }}
    out.unlockFocus();let rep=NSBitmapImageRep(data:out.tiffRepresentation!)!
    try rep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:dir+name))
}
let cases=[("screenshot-dmin","Screenshot Dmin"),("clear-margin-dmin","Clear-film margin Dmin"),("whole-picture-dmin","Whole-picture Dmin estimate")]
try sheet([cases.map{("dog-"+$0.0,$0.1+"; nothing else changed")}],400,800.0/3,"comparison.png")
try sheet(["forehead","muzzle","chest"].map{r in cases.map{("\(r)-\($0.0)-unchanged-grade","\(r): \($0.1)")}},384,288,"fur-crops.png")
let crop=NSImage(contentsOfFile:dir+"whole-picture-base-source-inspection-plus2ev.tif")!
let out=NSImage(size:NSSize(width:416,height:352));out.lockFocus();crop.draw(in:NSRect(x:0,y:0,width:416,height:352))
NSColor.systemYellow.setStroke();let box=NSBezierPath(rect:NSRect(x:192,y:160,width:32,height:32));box.lineWidth=2;box.stroke()
("(5588,3780), 32 x 32" as NSString).draw(at:NSPoint(x:190,y:197),withAttributes:[.font:NSFont.systemFont(ofSize:12),.foregroundColor:NSColor.systemYellow])
out.unlockFocus();let rep=NSBitmapImageRep(data:out.tiffRepresentation!)!
try rep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:dir+"whole-picture-candidate.png"))
let full=NSImage(contentsOfFile:".cache/probes/dmin-trace/run1/source-full-inspection-plus2ev.tif")!
let context=NSImage(size:NSSize(width:1200,height:800));context.lockFocus();full.draw(in:NSRect(x:0,y:0,width:1200,height:800))
let x=Double(5588-12)/8640*1200,y=800-Double(3780-12)/5760*800
NSColor.systemYellow.setStroke();let mark=NSBezierPath(ovalIn:NSRect(x:x-8,y:y-8,width:16,height:16));mark.lineWidth=2;mark.stroke()
("Whole-picture estimate" as NSString).draw(at:NSPoint(x:x+12,y:y),withAttributes:[.font:NSFont.systemFont(ofSize:14),.foregroundColor:NSColor.systemYellow])
context.unlockFocus();let contextRep=NSBitmapImageRep(data:context.tiffRepresentation!)!
try contextRep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:dir+"whole-picture-position.png"))
