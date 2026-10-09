import AppKit
let dir=(CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ".cache/probes/film-model-images/run2-whole-dmin")+"/"
let dogOnly=CommandLine.arguments.contains("--dog-only")
func sheet(_ rows:[[(String,String)]],_ cell:NSSize,_ name:String) throws {
    let out=NSImage(size:NSSize(width:cell.width*3,height:(cell.height+25)*Double(rows.count)))
    out.lockFocus();NSColor(calibratedWhite:0.12,alpha:1).setFill();NSRect(origin:.zero,size:out.size).fill()
    for (j,row) in rows.enumerated() {for (i,item) in row.enumerated() {
        let x=Double(i)*cell.width,y=Double(rows.count-1-j)*(cell.height+25)
        guard let image=NSImage(contentsOfFile:dir+item.0) else {fatalError("missing image: \(item.0)")}
        image.draw(in:NSRect(x:x,y:y+25,width:cell.width,height:cell.height))
        if item.0.contains("base-context") {
            NSColor.systemYellow.setStroke()
            let box=NSBezierPath(rect:NSRect(x:x+160,y:y+25+64,width:128,height:32));box.lineWidth=2;box.stroke()
        }
        (item.1 as NSString).draw(at:NSPoint(x:x+6,y:y+5),withAttributes:[.font:NSFont.systemFont(ofSize:12),.foregroundColor:NSColor.white])
    }}
    out.unlockFocus();let rep=NSBitmapImageRep(data:out.tiffRepresentation!)!
    try rep.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:dir+name))
}
let modes=["current","fitted-toe","gentle-shoulder"]
let titles=["Current: fitted roll / matched mids","Toe: fitted roll / matched mids","Toe + shoulder: fitted roll / matched mids"]
try sheet((dogOnly ? [4393] : [4393,4399,4415]).map {id in modes.enumerated().map {i,m in ("DSC\(String(format:"%05d",id))-\(m)-matched.tif","\(id): \(titles[i])")}},NSSize(width:360,height:240),"comparison.png")
try sheet(["forehead","muzzle","chest","sky"].map {roi in modes.enumerated().map {i,m in ("DSC04393-\(m)-\(roi)-100pct.tif","\(roi): \(titles[i])")}},NSSize(width:384,height:288),"dog-crops.png")
try sheet(modes.enumerated().map {i,m in [-2,0,2].map {ev in ("DSC04393-\(m)-\(ev==0 ? "matched" : "ev\(ev>0 ? "+" : "")\(ev)").tif","\(titles[i]): \(ev==0 ? "matched" : "\(ev>0 ? "+" : "")\(ev) EV")")}},NSSize(width:360,height:240),"dog-exposure.png")
if !dogOnly {
    try sheet(["sunlit-canopy","sky-bokeh","leaf-shadow"].map {roi in modes.enumerated().map {i,m in ("DSC04415-\(m)-\(roi)-100pct.tif","\(roi): \(titles[i])")}},NSSize(width:384,height:288),"canopy-crops.png")
    try sheet([[4393,4399,4415].map {id in ("DSC\(String(format:"%05d",id))-base-context-source-plus2EV.tif","\(id): source base context, +2 EV view only") }],NSSize(width:512,height:216),"base-contexts.png")
}
