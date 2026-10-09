import AppKit
let dir=".cache/probes/drt-lift/"
func contact(_ names:[(String,String)], columns:Int, output:String) throws {
    let cellW=360,cellH=265,rows=(names.count+columns-1)/columns
    let sheet=NSImage(size:NSSize(width:columns*cellW,height:rows*cellH))
    sheet.lockFocus();NSColor(calibratedWhite:0.12,alpha:1).setFill();NSRect(x:0,y:0,width:columns*cellW,height:rows*cellH).fill()
    for (i,item) in names.enumerated() {
        let x=(i%columns)*cellW,y=(rows-i/columns-1)*cellH
        guard let image=NSImage(contentsOfFile:dir+item.0) else {continue}
        image.draw(in:NSRect(x:x,y:y+25,width:cellW,height:240))
        (item.1 as NSString).draw(at:NSPoint(x:x+7,y:y+5),withAttributes:[.font:NSFont.systemFont(ofSize:13),.foregroundColor:NSColor.white])
    }
    sheet.unlockFocus()
    let bitmap=NSBitmapImageRep(data:sheet.tiffRepresentation!)!
    try bitmap.representation(using:.png,properties:[:])!.write(to:URL(fileURLWithPath:dir+output))
}
try contact([
("DSC04393-current-matched.tif","Dog: current / EV +3"),
("DSC04393-lift-linear-matched.tif","Neutral luminance lift / EV +1.504"),
("DSC04393-lift-channel-matched.tif","Per-channel lift / EV +1.522"),
("DSC04399-current-matched.tif","Life ring: current / EV +3"),
("DSC04399-lift-linear-matched.tif","Neutral luminance lift / EV +2.008"),
("DSC04399-lift-channel-matched.tif","Per-channel lift / EV +2.062"),
("DSC04393-lift-density-matched.tif","Density-baseline lift / EV +0.292"),
("DSC04399-lift-density-matched.tif","Density-baseline lift / EV +0.790"),
("DSC04393-lift-linear-originalEV3.tif","Neutral lift: unchanged original EV +3")
],columns:3,output:"comparison.png")
try contact([
("DSC04393-current-minus1.tif","Current / EV +2"),
("DSC04393-current-matched.tif","Current / EV +3"),
("DSC04393-current-plus1.tif","Current / EV +4"),
("DSC04393-lift-linear-minus1.tif","Neutral lift / EV +0.504"),
("DSC04393-lift-linear-matched.tif","Neutral lift / EV +1.504"),
("DSC04393-lift-linear-plus1.tif","Neutral lift / EV +2.504")
],columns:3,output:"exposure-dog.png")
