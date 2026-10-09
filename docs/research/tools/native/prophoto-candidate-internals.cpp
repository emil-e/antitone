#include <OpenColorIO/OpenColorIO.h>
#include <cstdio>
#include "../../../../.cache/probes/prophoto-boundary-candidate.cpp"
namespace OCIO=OCIO_NAMESPACE;
using namespace OCIO;
using namespace OCIO::ACES2;
int main() {
 const Primaries primaries[]={REC709::primaries,{{.7347,.2653},{.1596,.8404},{.0366,.0001},{.3457,.3585}}};
 const char* names[]={"sRGB","ProPhoto"};
 auto in=init_JMhParams(ACES_AP0::primaries);auto reach=init_JMhParams(ACES_AP1::primaries);auto t=init_ToneScaleParams(100);auto s=init_SharedCompressionParams(100,in,reach);auto c=init_ChromaCompressParams(100,t);
 for(int i=0;i<2;++i) {
  auto out=init_JMhParams(primaries[i]);auto g=init_GamutCompressParams(100,in,out,t,s,reach);
  for(auto rgb:std::array<f3,6>{{{.4f,.4f,.2f},{.4f,.2f,.2f},{.4f,.4f,.4f},{.4f,.4f,0.f},{.4f,.4f,.1f},{.8f,.8f,.2f}}}) {
   auto ap0=mult_f3_f33(rgb,RGBtoRGB_f33(ACES_AP1::primaries,ACES_AP0::primaries));
   auto aab=RGB_to_Aab(ap0,in);auto jmh=Aab_to_JMh(aab,in);auto rp=resolve_CompressionParams(jmh[2],s);
   auto n=chroma_compress_norm(std::cos(to_radians(jmh[2])),std::sin(to_radians(jmh[2])),c.chroma_compress_scale);
   auto jmht=chroma_compress_fwd(jmh,tonescale_A_to_J_fwd(aab[0],in,t),n,rp,c);
   auto hdp=init_HueDependantGamutParams(jmh[2],rp,g);
   auto sg=get_focus_gain(jmht[0],hdp.analytical_threshold,rp.limit_J_max,g.focus_dist);
   auto js=solve_J_intersect(jmht[0],jmht[1],hdp.focusJ,rp.limit_J_max,sg);
   auto slope=compute_compression_vector_slope(js,hdp.focusJ,rp.limit_J_max,sg);
   auto jc=solve_J_intersect(hdp.JMcusp[0],hdp.JMcusp[1],hdp.focusJ,rp.limit_J_max,sg);
   auto lower=estimate_line_and_boundary_intersection_M(js,slope,hdp.gamma_bottom_inv,hdp.JMcusp[0],hdp.JMcusp[1],jc);
   auto upper=estimate_line_and_boundary_intersection_M(rp.limit_J_max-js,-slope,hdp.gamma_top_inv,rp.limit_J_max-hdp.JMcusp[0],hdp.JMcusp[1],rp.limit_J_max-jc);
   auto boundary=find_gamut_boundary_intersection(hdp.JMcusp,rp.limit_J_max,hdp.gamma_top_inv,hdp.gamma_bottom_inv,js,slope,jc);
   auto reachBoundary=estimate_line_and_boundary_intersection_M(js,slope,rp.model_gamma_inv,rp.limit_J_max,rp.reachMaxM,rp.limit_J_max);
   auto mapped=gamut_compress_fwd(jmht,rp,g);auto rgbout=JMh_to_RGB(mapped,out);
   std::printf("%s input=(%.3g %.3g %.3g) JMh=(%.9g %.9g %.9g) toneJMh=(%.9g %.9g %.9g) cusp=(%.9g %.9g) gamma_inv=(%.9g %.9g) focus=%.9g js=%.9g jc=%.9g slope=%.9g lower=%.9g upper=%.9g boundary=%.9g reach=%.9g mapped=(%.9g %.9g %.9g) RGB=(%.9g %.9g %.9g)\n",names[i],rgb[0],rgb[1],rgb[2],jmh[0],jmh[1],jmh[2],jmht[0],jmht[1],jmht[2],hdp.JMcusp[0],hdp.JMcusp[1],hdp.gamma_bottom_inv,hdp.gamma_top_inv,hdp.focusJ,js,jc,slope,lower,upper,boundary,reachBoundary,mapped[0],mapped[1],mapped[2],rgbout[0],rgbout[1],rgbout[2]);
  }
 }
}
