#include <OpenColorIO/OpenColorIO.h>
#include <cstdio>
#include <cstdlib>
#include "../../../../.cache/deps/OpenColorIO-2.6.0/src/OpenColorIO/ops/fixedfunction/ACES2/Transform.cpp"
namespace OCIO=OCIO_NAMESPACE;
using namespace OCIO;
using namespace OCIO::ACES2;
int main(int argc,char** argv) {
 const float peak=argc>1?std::strtof(argv[1],nullptr):100.f;
 auto in=init_JMhParams(ACES_AP0::primaries);auto reach=init_JMhParams(ACES_AP1::primaries);auto t=init_ToneScaleParams(peak);auto s=init_SharedCompressionParams(peak,in,reach);auto c=init_ChromaCompressParams(peak,t);
 const Primaries targets[]={REC709::primaries,REC2020::primaries,P3_D65::primaries};
 const char* names[]={"sRGB","Rec2020","P3-D65"};
 const float levels[]={0,.0001f,.001f,.01f,.05f,.1f,.18f,.2f,.4f,.5f,1,2,4,16,64,1024};
 int failures=0;
 for(unsigned target=0;target<3;++target) {
  auto out=init_JMhParams(targets[target]);auto g=init_GamutCompressParams(peak,in,out,t,s,reach);
  unsigned bad=0,checked=0;
  for(float r:levels)for(float gg:levels)for(float b:levels) {
   const f3 rgb={r,gg,b};auto ap0=mult_f3_f33(rgb,RGBtoRGB_f33(ACES_AP1::primaries,ACES_AP0::primaries));auto aab=RGB_to_Aab(ap0,in);auto jmh=Aab_to_JMh(aab,in);if(jmh[0]<=0)continue;
   auto rp=resolve_CompressionParams(jmh[2],s);auto n=chroma_compress_norm(std::cos(to_radians(jmh[2])),std::sin(to_radians(jmh[2])),c.chroma_compress_scale);
   auto jmht=chroma_compress_fwd(jmh,tonescale_A_to_J_fwd(aab[0],in,t),n,rp,c);
   if(jmht[0]<=0||jmht[0]>=rp.limit_J_max||jmht[1]<.01f)continue;
   ++checked;auto mapped=gamut_compress_fwd(jmht,rp,g);auto rendered=JMh_to_RGB(mapped,out);
   if(mapped[1]<=0.f||!std::isfinite(rendered[0])||!std::isfinite(rendered[1])||!std::isfinite(rendered[2])) {
    if(bad<10)std::printf("target=%s bad AP1=(%.9g %.9g %.9g) toneJMh=(%.9g %.9g %.9g) mappedM=%.9g\n",names[target],r,gg,b,jmht[0],jmht[1],jmht[2],mapped[1]);++bad;
   }
  }
  std::printf("target=%s peak=%.9g checked=%u zero_chroma_or_nonfinite=%u\n",names[target],peak,checked,bad);failures+=bad;
 }
 return failures?1:0;
}
