import AppKit
let dir=".cache/probes/dmin-trace/run1/"
for name in ["full","top-centre","bottom-centre","left","right"] {
    let image=NSImage(contentsOfFile:dir+"source-\(name)-inspection-plus2ev.tif")!
    let size = name == "full" ? NSSize(width:1200,height:800) : (name == "top-centre" || name == "bottom-centre" ? NSSize(width:1024,height:312) : NSSize(width:400,height:800))
    let out=NSImage(size:size);out.lockFocus();image.draw(in:NSRect(origin:.zero,size:size));out.unlockFocus()
    let rep=NSBitmapImageRep(data:out.tiffRepresentation!)!
    try rep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:dir+"source-\(name).png"))
}
