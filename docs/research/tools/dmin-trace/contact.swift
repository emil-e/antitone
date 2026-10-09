import AppKit
let dir=".cache/probes/dmin-trace/run1/"
func sheet(_ rows:[[(String,String)]],_ width:Double,_ height:Double,_ name:String) throws {
    let size=NSSize(width:width*3,height:(height+26)*Double(rows.count))
    let out=NSImage(size:size);out.lockFocus();NSColor(calibratedWhite:0.12,alpha:1).setFill();NSRect(origin:.zero,size:size).fill()
    for (j,row) in rows.enumerated(){for(i,item) in row.enumerated(){
        let x=Double(i)*width,y=Double(rows.count-1-j)*(height+26)
        let image=NSImage(contentsOfFile:dir+item.0+".tif")!
        image.draw(in:NSRect(x:x,y:y+26,width:width,height:height))
        (item.1 as NSString).draw(at:NSPoint(x:x+6,y:y+5),withAttributes:[.font:NSFont.systemFont(ofSize:12),.foregroundColor:NSColor.white])
    }}
    out.unlockFocus();let rep=NSBitmapImageRep(data:out.tiffRepresentation!)!
    try rep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:dir+name))
}
try sheet([[ ("dog-screenshot-dmin","Screenshot Dmin; current model"),("dog-verified-dmin","Measured Dmin ONLY; current model"),("dog-verified-dmin-plus2EV","Measured Dmin +2 EV; current model") ]],400,800.0/3,"comparison.png")
try sheet(["forehead","muzzle","chest"].map {r in [
    ("\(r)-screenshot-dmin-unchanged-grade","\(r): screenshot Dmin"),
    ("\(r)-verified-dmin-unchanged-grade","\(r): measured Dmin ONLY"),
    ("\(r)-verified-dmin-plus2EV","\(r): measured Dmin +2 EV")
]},384,288,"fur-crops.png")
let image=NSImage(contentsOfFile:dir+"verified-base-source-inspection-plus2ev.tif")!
let out=NSImage(size:NSSize(width:512,height:216));out.lockFocus();image.draw(in:NSRect(x:0,y:0,width:512,height:216))
NSColor.systemYellow.setStroke();let box=NSBezierPath(rect:NSRect(x:160,y:64,width:128,height:32));box.lineWidth=2;box.stroke()
("Dmin: (5000,5660) 128 x 32" as NSString).draw(at:NSPoint(x:160,y:100),withAttributes:[.font:NSFont.systemFont(ofSize:12),.foregroundColor:NSColor.systemYellow])
out.unlockFocus();let rep=NSBitmapImageRep(data:out.tiffRepresentation!)!
try rep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:dir+"verified-base.png"))
