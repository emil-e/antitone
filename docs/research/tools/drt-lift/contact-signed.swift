import AppKit
let dir=".cache/probes/drt-lift/run3/"
let names:[(String,String)]=[
("DSC04393-current-matched.tif","Dog: current / EV +3"),
("DSC04393-lift-radius-toe-matched.tif","Signed-safe toe lift / EV +1.531"),
("DSC04393-lift-radius-black-matched.tif","Same lift + explicit black shift 0.001"),
("DSC04399-current-matched.tif","Life ring: current / EV +3"),
("DSC04399-lift-radius-toe-matched.tif","Signed-safe toe lift / EV +2.072"),
("DSC04399-lift-radius-black-matched.tif","Same lift + explicit black shift 0.001")]
let sheet=NSImage(size:NSSize(width:1080,height:530))
sheet.lockFocus();NSColor(calibratedWhite:0.12,alpha:1).setFill();NSRect(x:0,y:0,width:1080,height:530).fill()
for (i,item) in names.enumerated() {
    let x=(i%3)*360,y=(1-i/3)*265
    guard let image=NSImage(contentsOfFile:dir+item.0) else {fatalError("missing image")}
    image.draw(in:NSRect(x:x,y:y+25,width:360,height:240))
    (item.1 as NSString).draw(at:NSPoint(x:x+6,y:y+5),withAttributes:[.font:NSFont.systemFont(ofSize:12),.foregroundColor:NSColor.white])
}
sheet.unlockFocus()
let bitmap=NSBitmapImageRep(data:sheet.tiffRepresentation!)!
try bitmap.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:dir+"comparison-signed.png"))
