
// Declaration of all textures

layout(set=0, binding = 1) uniform sampler2D aces_reach_m_table_0Sampler; 
layout(set=0, binding = 2) uniform sampler2D aces_gamut_cusp_table_0Sampler; 

// Declaration of all helper methods

float aces_reach_m_table_0_sample(float h)
{
  float i_base = floor(h);
  float i_lo = i_base + float(1);
  float i_hi = i_lo + 1.0;
  float lo = texture(aces_reach_m_table_0Sampler, vec2((i_lo + 0.5) / float (363), 0.0)).r;
  float hi = texture(aces_reach_m_table_0Sampler, vec2((i_hi + 0.5) / float (363), 0.5)).r;
  float t = h - i_base;
  return mix(lo, hi, t);
}
float aces_tonescale_fwd0(float J)
{
  float A = 0.0323680267 * pow(abs(J) * 0.00999999978, 0.879464149);
  float Y = pow(( 27.1299992 * A) / (1.0f - A), 2.3809523809523809);
  float f = 10.1729107 * pow(Y / (Y + 4.6796021847256952), 1.14999998);
  float Y_ts = max(0.0, f * f / (f + 0.0399999991));
  float F_L_Y = pow(0.79370057210326195 * Y_ts, 0.42);
  float J_ts = 100. * pow((F_L_Y / ( 27.1299992 + F_L_Y)) * 30.8946857, 1.13705599);
  return sign(J) * J_ts;
}
float aces_toe_fwd0(float x, float limit, float k1_in, float k2_in)
{
  float k2 = max(k2_in, 0.001);
  float k1 = sqrt(k1_in * k1_in + k2 * k2);
  float k3 = (limit + k1) / (limit + k2);
  return (x > limit) ? x : 0.5 * (k3 * x - k1 + sqrt((k3 * x - k1) * (k3 * x - k1) + 4.0 * k2 * k3 * x));
}
const float aces_gamut_cusp_table_0_hues_array[363] = float[363](-1.00411987, 0., 0.979170561, 1.95834112, 2.93751168, 3.91668224, 4.89585304, 5.87502337, 6.85419369, 7.83336449, 8.81253529, 9.79170609, 10.7708759, 11.7500467, 12.7292175, 13.7083874, 14.6875582, 15.666729, 16.6458988, 17.6250706, 18.6042404, 19.5834122, 20.562582, 21.5417519, 22.5209236, 23.5000935, 24.7101231, 25.9201527, 26.9326515, 27.9451504, 28.9576492, 29.9701481, 30.9826469, 31.9951458, 33.0076447, 34.0201454, 35.0326424, 36.0451431, 37.0576401, 38.0701408, 39.0826378, 40.0951385, 41.1076355, 42.1201363, 43.132637, 44.145134, 45.1576347, 46.1701317, 47.1826324, 48.1951294, 49.2076302, 50.2201271, 51.2326279, 52.2451248, 53.2576256, 54.2701225, 55.2826233, 56.2951202, 57.307621, 58.3201218, 59.3326187, 60.3451195, 61.3576164, 62.3701172, 63.3826141, 64.3951111, 65.4076157, 66.4201126, 67.4326096, 68.4451065, 69.4576111, 70.470108, 71.482605, 72.4951019, 73.5076065, 74.5201035, 75.5326004, 76.545105, 77.5576019, 78.5700989, 79.5825958, 80.5951004, 81.6075974, 82.6200943, 83.6325912, 84.6450958, 85.6575928, 86.6700897, 87.6825867, 88.6950912, 89.7075882, 90.7200851, 91.7325897, 92.7450867, 93.7575836, 94.7700806, 95.7825851, 96.7950821, 97.807579, 98.820076, 99.8325806, 100.845078, 101.857574, 102.870071, 103.882576, 104.895073, 105.90757, 106.317032, 107.297791, 108.278542, 109.2593, 110.240051, 111.22081, 112.201561, 113.18232, 114.163071, 115.143829, 116.124588, 117.105339, 118.086098, 119.066849, 120.047607, 121.028358, 122.009117, 122.989876, 123.970627, 124.951385, 125.932137, 126.912895, 127.893646, 128.874405, 129.855164, 130.835922, 131.816666, 132.797424, 133.778183, 134.758942, 135.739685, 136.720444, 137.701202, 138.681961, 139.66272, 140.643463, 141.624222, 142.60498, 143.585739, 144.566483, 145.547241, 146.528, 147.764053, 148.793884, 149.8237, 150.853531, 151.883362, 152.913177, 153.943008, 154.972839, 156.002655, 157.032486, 158.062317, 159.092133, 160.121964, 161.151794, 162.18161, 163.211441, 164.241272, 165.271088, 166.300919, 167.33075, 168.360565, 169.390396, 170.420227, 171.450043, 172.479874, 173.509705, 174.53952, 175.569351, 176.599182, 177.628998, 178.658829, 179.68866, 180.718475, 181.748306, 182.778137, 183.807968, 184.837784, 185.867615, 186.897446, 187.927261, 188.957092, 189.986923, 191.016739, 192.04657, 192.160278, 193.156769, 194.153275, 195.149765, 196.146271, 197.142761, 198.139252, 199.135757, 200.132248, 201.128754, 202.125244, 203.121735, 204.11824, 205.114731, 206.111237, 207.107727, 208.104218, 209.100723, 210.097214, 211.093719, 212.09021, 213.0867, 214.083206, 215.079697, 216.076202, 217.072693, 218.069183, 219.065689, 220.06218, 221.058685, 222.055176, 223.051666, 224.048172, 225.044662, 226.041168, 227.037659, 228.034149, 229.030655, 230.027145, 231.023651, 232.020142, 233.016632, 234.013138, 235.009628, 236.006134, 237.002625, 237.999115, 238.995621, 239.992111, 240.988617, 241.985107, 242.981598, 243.978104, 244.974594, 245.9711, 246.96759, 247.964081, 248.960587, 249.957077, 250.953583, 251.950073, 252.946564, 253.943069, 254.93956, 255.936066, 256.932556, 257.929047, 258.925537, 259.922058, 260.918549, 261.915039, 262.91153, 263.90802, 264.904541, 265.901031, 266.897522, 267.929382, 268.961243, 269.970245, 270.979218, 271.98822, 272.997192, 274.006195, 275.015167, 276.02417, 277.033142, 278.042145, 279.051117, 280.06012, 281.069122, 282.078094, 283.087097, 284.096069, 285.105072, 286.114044, 287.123047, 288.132019, 289.141022, 290.149994, 291.158997, 292.167969, 293.176971, 294.185974, 295.194946, 296.203949, 297.212921, 298.221924, 299.230896, 300.239899, 301.248871, 302.257874, 303.266846, 304.275848, 305.284851, 306.293823, 307.302826, 308.311798, 309.320801, 310.329773, 311.338776, 312.347748, 313.35675, 314.365723, 315.374725, 316.383728, 317.3927, 318.401703, 319.410675, 320.419678, 321.42865, 322.437653, 323.446625, 324.455627, 325.4646, 326.473602, 327.482574, 328.491577, 329.50058, 330.509552, 331.518555, 332.527527, 333.53653, 334.545502, 335.554504, 336.563477, 337.572479, 338.581451, 339.590454, 340.599457, 341.608429, 341.925629, 342.929749, 343.933899, 344.938019, 345.942169, 346.946289, 347.950409, 348.954559, 349.958679, 350.96283, 351.966949, 352.971069, 353.97522, 354.97934, 355.983459, 356.98761, 357.99173, 358.99588, 360., 360.979156);
vec3 aces_gamut_cusp_table_0_sample(float h)
{
  int i = int(h) + 1;
  int i_lo = int(max(float(0), float(i + -1)));
  int i_hi = int(min(float(361), float(i + 2)));
  while (i_lo + 1 < i_hi)
  {
    float hcur = aces_gamut_cusp_table_0_hues_array[i];
    if (h > hcur)
    {
      i_lo = i;
    }
    else
    {
      i_hi = i;
    }
    i = (i_lo + i_hi) / 2;
  }
  vec3 lo = texture(aces_gamut_cusp_table_0Sampler, vec2((float(i_hi) - 1.0 + 0.5) / float(363), 0.5)).rgb;
  vec3 hi = texture(aces_gamut_cusp_table_0Sampler, vec2((float(i_hi) + 0.5) / float(363), 0.5)).rgb;
  float t = (h - aces_gamut_cusp_table_0_hues_array[i_hi - 1]) / (aces_gamut_cusp_table_0_hues_array[i_hi] - aces_gamut_cusp_table_0_hues_array[i_hi - 1]);
  return mix(lo, hi, t);
}
float aces_get_focus_gain0(float J, float cuspJ)
{
  float thr = mix(cuspJ, 283.249878, 0.300000);
  if (J > thr)
  {
    float gain = ( 283.249878 - thr) / max(0.0001, 283.249878 - J);
    gain = log(gain)/log(10.0);
    return gain * gain + 1.0;
  }
  else
  {
    return 1.0;
  }
}
float aces_solve_J_intersect0(float J, float M, float focusJ, float slope_gain)
{
  float M_scaled = M / slope_gain;
  float a = M_scaled / focusJ;
  if (J < focusJ)
  {
    float b = 1.0 - M_scaled;
    float c = -J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b + root);
  }
  else
  {
    float b = - (1.0 + M_scaled + 283.249878 * a);
    float c = 283.249878 * M_scaled + J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b - root);
  }
}
float aces_find_gamut_boundary_intersection0(vec2 JM_cusp, float gamma_top_inv, float gamma_bottom_inv, float J_intersect_source, float J_intersect_cusp, float slope)
{
  float M_boundary_lower = J_intersect_cusp * pow(J_intersect_source / J_intersect_cusp, gamma_bottom_inv) / (JM_cusp.r / JM_cusp.g - slope);
  float M_boundary_upper = JM_cusp.g * (283.249878 - J_intersect_cusp) * pow((283.249878 - J_intersect_source) / (283.249878 - J_intersect_cusp), gamma_top_inv) / (slope * JM_cusp.g + 283.249878 - JM_cusp.r);
  float smin = 0.0;
  {
    float a = M_boundary_lower;
    float b = M_boundary_upper;
    float s = 0.119999997 * JM_cusp.g;
    float h = max(s - abs(a - b), 0.0) / s;
    smin = min(a, b) - h * h * h * s * 0.16666666666666666;
  }
  return smin;
}
float aces_remap_M_fwd0(float M, float gamut_boundary_M, float reach_boundary_M)
{
  float boundary_ratio = gamut_boundary_M / reach_boundary_M;
  float proportion = max(boundary_ratio, 0.75);
  float threshold = proportion * gamut_boundary_M;
  if (proportion >= 1.0f || M <= threshold)
  {
    return M;
  }
  float m_offset = M - threshold;
  float gamut_offset = gamut_boundary_M - threshold;
  float reach_offset = reach_boundary_M - threshold;
  float scale = reach_offset / ((reach_offset / gamut_offset) - 1.0f);
  float nd = m_offset / scale;
  return threshold + scale * nd / (1.0f + nd);
}
vec3 aces_gamut_compress0(vec3 JMh, float Jx, vec3 JMGcusp, float reachMaxM)
{
  float J = JMh.r;
  float M = JMh.g;
  float h = JMh.b;
  if (M <= 0.0 || J > 283.249878)
  {
    return vec3(J, 0.0, h);
  }
  else
  {
    vec2 JMcusp = JMGcusp.rg;
    float focusJ = mix(JMcusp.r, 40.816883, min(1.0, 1.300000 - (JMcusp.r / 283.249878)));
    float slope_gain = 1051.56519 * aces_get_focus_gain0(Jx, JMcusp.r);
    float J_intersect_source = aces_solve_J_intersect0(JMh.r, JMh.g, focusJ, slope_gain);
    float gamut_slope = (J_intersect_source < focusJ) ? J_intersect_source : (283.249878 - J_intersect_source);
    gamut_slope = gamut_slope * (J_intersect_source - focusJ) / (focusJ * slope_gain);
    float gamma_top_inv = JMGcusp.b;
    float gamma_bottom_inv = 0.826446235;
    float J_intersect_cusp = aces_solve_J_intersect0(JMcusp.r, JMcusp.g, focusJ, slope_gain);
    float gamutBoundaryM = aces_find_gamut_boundary_intersection0(JMcusp, gamma_top_inv, gamma_bottom_inv, J_intersect_source, J_intersect_cusp, gamut_slope);
    if (gamutBoundaryM <= 0.0)
    {
      return vec3(J, 0.0, h);
    }
    float reachBoundaryM = 283.249878 * pow(J_intersect_source / 283.249878,  0.879464149);
    reachBoundaryM = reachBoundaryM / ((283.249878 / reachMaxM) - gamut_slope);
    float remapped_M = aces_remap_M_fwd0(M, gamutBoundaryM, reachBoundaryM);
    float remapped_J = J_intersect_source + remapped_M * gamut_slope;
    return vec3(remapped_J, remapped_M, h);
  }
}

// Declaration of the OCIO shader function

vec4 antitone_render(vec4 inPixel)
{
  vec4 outColor = inPixel;
  
  // Add Range processing
  
  {
    outColor.rgb = max(vec3(0., 0., 0.), outColor.rgb);
    outColor.rgb = min(vec3(1024., 1024., 1024.), outColor.rgb);
  }
  
  // Add Matrix processing
  
  {
    vec4 res = vec4(outColor.rgb.r, outColor.rgb.g, outColor.rgb.b, outColor.a);
    vec4 tmp = res;
    res = mat4(0.69545224135745187, 0.044794563372037723, -0.0055258825581135443, 0., 0.14067869647029416, 0.85967111845642175, 0.0040252103059786595, 0., 0.16386906217225405, 0.0955343181715404, 1.0015006722521349, 0., 0., 0., 0., 1.) * tmp;
    outColor.rgb = vec3(res.x, res.y, res.z);
    outColor.a = res.w;
  }
  
  // Add FixedFunction 'ACES_OutputTransform20 (Forward)' processing
  
  {
    
    // Add RGB to JMh
    
    vec3 JMh;
    vec3 Aab;
    {
      {
        vec3 lms = mat3(0.445181042, 0.123734146, 0.0117007261, 0.34964928, 0.613643706, 0.0280607939, -0.00112973328, 0.0563228093, 0.753939092) * outColor.rgb;
        vec3 F_L_v = pow(abs(lms), vec3(0.419999987, 0.419999987, 0.419999987));
        vec3 rgb_a = (sign(lms) * F_L_v) / ( 27.1299992 + F_L_v);
        Aab = mat3(20.2588081, 15480., 1720., 10.1294041, -16887.2734, 1720., 0.506470203, 1407.27271, -3440.) * rgb_a.rgb;
      }
      {
        if (Aab.r <= 0.0)
        {
          JMh.rgb = vec3(0., 0., 0.);
        }
        else
        {
          float J = 100. * pow(Aab.r, 1.13705599);
          float M = (J == 0.0) ? 0.0 : sqrt(Aab.g * Aab.g + Aab.b * Aab.b);
          float h = (Aab.g == 0.0) ? 0.0 : atan(Aab.b, Aab.g) * 57.29577951308238;
          h = h - floor(h / 360.0) * 360.0;
          h = (h < 0.0) ? h + 360.0 : h;
          JMh.rgb = vec3(J, M, h);
        }
      }
      outColor.rgb = JMh;
    }
    float h_rad = outColor.b * 0.0174532924;
    float cos_hr = cos(h_rad);
    float sin_hr = sin(h_rad);
    
    // Add ToneScale and ChromaCompress (fwd)
    
    float J_ts = aces_tonescale_fwd0(outColor.r);
    // Sample tables (fwd)
    float reachMaxM = aces_reach_m_table_0_sample(outColor.b);
    
    {
      float J = outColor.r;
      float M = outColor.g;
      float h = outColor.b;
      float M_cp = M;
      if (M != 0.0)
      {
        float nJ = J_ts / 283.249878;
        float snJ = max(0.0, 1.0 - nJ);
        float Mnorm;
        {
          float cos_hr2 = 2.0 * cos_hr * cos_hr - 1.0;
          float sin_hr2 = 2.0 * cos_hr * sin_hr;
          float cos_hr3 = 4.0 * cos_hr * cos_hr * cos_hr - 3.0 * cos_hr;
          float sin_hr3 = 3.0 * sin_hr - 4.0 * sin_hr * sin_hr * sin_hr;
          vec3 cosines = vec3(cos_hr, cos_hr2, cos_hr3);
          vec3 cosine_weights = vec3(28.177105004310604, 40.918782982468606, 19.588056175708772);
          vec3 sines = vec3(sin_hr, sin_hr2, sin_hr3);
          vec3 sine_weights = vec3(36.435131137728689, -15.832440585136412, 22.842479106426239);
          Mnorm = dot(cosines, cosine_weights) + dot(sines, sine_weights) + 191.63428819274904;
        }
        float limit = pow(nJ, 0.879464149) * reachMaxM / Mnorm;
        M_cp = M * pow(J_ts / J, 0.879464149);
        M_cp = M_cp / Mnorm;
        M_cp = limit - aces_toe_fwd0(limit - M_cp, limit - 0.001, snJ * 0.402999997, sqrt(nJ * nJ + 0.000500000024));
        M_cp = aces_toe_fwd0(M_cp, limit, nJ * 10.3199997, snJ);
        M_cp = M_cp * Mnorm;
      }
      outColor.rgb = vec3(J_ts, M_cp, h);
    }
    
    // Add GamutCompress (fwd)
    
    {
      vec3 JMGcusp = aces_gamut_cusp_table_0_sample(outColor.b);
      outColor.rgb = aces_gamut_compress0(outColor.rgb, outColor.r, JMGcusp, reachMaxM);
    }
    
    // Add JMh to RGB
    
    {
      vec3 JMh = outColor.rgb;
      vec3 Aab;
      {
        Aab.r = pow(JMh.r * 0.00999999978, 0.879464149);
        Aab.g = JMh.g * cos_hr;
        Aab.b = JMh.g * sin_hr;
      }
      {
        vec3 rgb_a = mat3(0.0323680304, 0.0323680304, 0.0323680304, 2.07657631e-05, -4.10250432e-05, -1.01296409e-05, 1.3260621e-05, -1.20174373e-05, -0.000290076074) * Aab.rgb;
        vec3 rgb_a_lim = min( abs(rgb_a), vec3(0.99000001, 0.99000001, 0.99000001) );
        vec3 lms = sign(rgb_a) * pow( 27.1299992 * rgb_a_lim / (1.0f - rgb_a_lim), vec3(2.38095236, 2.38095236, 2.38095236));
        JMh.rgb = mat3(4.18920946, -0.841454685, 0.00181781233, -2.83307672, 2.44025636, -0.111337893, -0.0962110385, -0.338880897, 1.36944115) * lms;
      }
      outColor.rgb = JMh;
    }
  }
  
  // Add Range processing
  
  {
    outColor.rgb = max(vec3(0., 0., 0.), outColor.rgb);
    outColor.rgb = min(vec3(10., 10., 10.), outColor.rgb);
  }

  return outColor;
}
