#include "../../../../native/rendering_transform.h"
#include <lcms2.h>
int main() {
    const auto &p=antitone::output_parameters(1);
    cmsCIExyY white={p[7],p[8],1.};
    cmsCIExyYTRIPLE primaries={{p[1],p[2],1.},{p[3],p[4],1.},{p[5],p[6],1.}};
    double params[]={2.4,1./1.055,.055/1.055,1./12.92,.04045};
    cmsToneCurve *curve=cmsBuildParametricToneCurve(nullptr,4,params);
    if(!curve)return 1;
    cmsToneCurve *curves[]={curve,curve,curve};
    cmsHPROFILE profile=cmsCreateRGBProfile(&white,&primaries,curves);
    if(!profile)return 2;
    cmsMLU *text=cmsMLUalloc(nullptr,1);
    if(!text||!cmsMLUsetASCII(text,"en","US","Antitone Display P3 v1")||!cmsWriteTag(profile,cmsSigProfileDescriptionTag,text))return 3;
    cmsSetProfileVersion(profile,4.3);
    cmsSetHeaderRenderingIntent(profile,INTENT_RELATIVE_COLORIMETRIC);
    if(!cmsMD5computeID(profile)||!cmsSaveProfileToFile(profile,"assets/profiles/antitone-display-p3-v1.icc"))return 4;
    cmsMLUfree(text);cmsCloseProfile(profile);cmsFreeToneCurve(curve);
}
