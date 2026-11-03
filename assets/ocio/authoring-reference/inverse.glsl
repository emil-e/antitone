
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
float aces_tonescale_inv0(float J)
{
  float A = 0.0323680267 * pow(abs(J) * 0.00999999978, 0.879464149);
  float Y = pow(( 27.1299992 * A) / (1.0f - A), 2.3809523809523809);
  float Y_i = Y / 0.79370057210326195;
  float Z = max(0.0, min(1.00826871, Y_i));
  float ht = 0.5 * (Z + sqrt(Z * (0.15999999642372131 + Z)));
  float Yo = 0.73009213709383403 / (pow((1.04710376 / ht), (0.86956523541917463)) - 1.0);
  float F_L_Y = pow(abs(Yo), 0.42);
  float J_ts = 100. * pow((F_L_Y / ( 27.1299992 + F_L_Y)) * 30.8946857, 1.13705599);
  return sign(J) * J_ts;
}
const float aces_gamut_cusp_table_0_hues_array[363] = float[363](-1.01858521, 0., 0.999435902, 1.9988718, 2.9983077, 3.99774361, 4.99717951, 5.99661541, 6.99605131, 7.99548721, 8.99492264, 9.99435902, 10.9937954, 11.9932308, 12.9926662, 13.9921026, 14.991539, 15.9909744, 16.9904099, 17.9898453, 18.9892826, 19.988718, 20.9881535, 21.9875908, 22.9870262, 23.9864616, 24.9858971, 25.5228806, 26.5427036, 27.5625267, 28.5823517, 29.6021748, 30.6219978, 31.6418209, 32.661644, 33.681469, 34.7012901, 35.7211151, 36.7409401, 37.7607613, 38.7805862, 39.8004074, 40.8202324, 41.8400574, 42.8598785, 43.8797035, 44.8995247, 45.9193497, 46.9391708, 47.9589958, 48.9788208, 49.998642, 51.0184669, 52.0382881, 53.0581131, 54.0779381, 55.0977592, 56.1175842, 57.1374054, 58.1572304, 59.1770554, 60.1968765, 61.2167015, 62.2365227, 63.2563477, 64.2761688, 65.2959976, 66.3158188, 67.33564, 68.3554611, 69.3752899, 70.3951111, 71.4149323, 72.434761, 73.4545822, 74.4744034, 75.4942322, 76.5140533, 77.5338745, 78.5536957, 79.5735245, 80.5933456, 81.6131668, 82.6329956, 83.6528168, 84.6726379, 85.6924667, 86.7122879, 87.7321091, 88.7519302, 89.771759, 90.7915802, 91.8114014, 92.8312302, 93.8510513, 94.8708725, 95.8906937, 96.9105225, 97.9303436, 98.9501648, 99.9699936, 100.989815, 102.009636, 103.029465, 104.049286, 105.069107, 106.088928, 106.417801, 107.398407, 108.379021, 109.359627, 110.34024, 111.320847, 112.30146, 113.282066, 114.26268, 115.243286, 116.2239, 117.204506, 118.185112, 119.165726, 120.146332, 121.126945, 122.107552, 123.088165, 124.068771, 125.049385, 126.029991, 127.010605, 127.991211, 128.971817, 129.952423, 130.933044, 131.913651, 132.894257, 133.874863, 134.855484, 135.83609, 136.816696, 137.797302, 138.777908, 139.75853, 140.739136, 141.719742, 142.700348, 143.680969, 144.661575, 145.642181, 147.207565, 148.207993, 149.20842, 150.208847, 151.209274, 152.209702, 153.210129, 154.210556, 155.210983, 156.211411, 157.211838, 158.212265, 159.212692, 160.21312, 161.213547, 162.213974, 163.214401, 164.214828, 165.215256, 166.215683, 167.21611, 168.216537, 169.216965, 170.217392, 171.217819, 172.218246, 173.218674, 174.219101, 175.219528, 176.219955, 177.220383, 178.22081, 179.221237, 180.221664, 181.222092, 182.222519, 183.222946, 184.223373, 185.223801, 186.224228, 187.224655, 188.225082, 189.22551, 190.225937, 191.226364, 192.226791, 193.271332, 194.271347, 195.271347, 196.271362, 197.271378, 198.271393, 199.271393, 200.271408, 201.271423, 202.271423, 203.271439, 204.271454, 205.271469, 206.271469, 207.271484, 208.2715, 209.271515, 210.271515, 211.27153, 212.271545, 213.271545, 214.271561, 215.271576, 216.271591, 217.271591, 218.271606, 219.271622, 220.271622, 221.271637, 222.271652, 223.271667, 224.271667, 225.271683, 226.271698, 227.271698, 228.271713, 229.271729, 230.271744, 231.271744, 232.271759, 233.271774, 234.271774, 235.27179, 236.271805, 237.27182, 238.27182, 239.271835, 240.271851, 241.271851, 242.271866, 243.271881, 244.271896, 245.271896, 246.271912, 247.271927, 248.271942, 249.271942, 250.271957, 251.271973, 252.271973, 253.271988, 254.272003, 255.272018, 256.272034, 257.272034, 258.272034, 259.272064, 260.272064, 261.272064, 262.272095, 263.272095, 264.272125, 265.272125, 266.272125, 267.272156, 268.272156, 269.199829, 270.127533, 271.055206, 271.98291, 272.910583, 273.915375, 274.920135, 275.924927, 276.929688, 277.934479, 278.93924, 279.944031, 280.948792, 281.953583, 282.958344, 283.963135, 284.967896, 285.972687, 286.977448, 287.982239, 288.987, 289.991791, 290.996552, 292.001343, 293.006104, 294.010895, 295.015656, 296.020447, 297.025208, 298.029999, 299.03476, 300.039551, 301.044312, 302.049103, 303.053864, 304.058655, 305.063416, 306.068207, 307.072998, 308.077759, 309.08255, 310.087311, 311.092102, 312.096863, 313.101654, 314.106415, 315.111206, 316.115967, 317.120758, 318.125519, 319.13031, 320.135071, 321.139862, 322.144623, 323.149414, 324.154175, 325.158966, 326.163727, 327.168518, 328.173279, 329.17807, 330.182831, 331.187622, 332.192383, 333.197174, 334.201935, 335.206726, 336.211487, 337.216278, 338.221069, 339.22583, 340.039001, 340.852203, 341.665375, 342.68396, 343.702545, 344.721161, 345.739746, 346.758331, 347.776917, 348.795502, 349.814087, 350.832703, 351.851288, 352.869873, 353.888458, 354.907043, 355.925629, 356.944244, 357.96283, 358.981415, 360., 360.999451);
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
  float thr = mix(cuspJ, 100.000000, 0.300000);
  if (J > thr)
  {
    float gain = ( 100. - thr) / max(0.0001, 100. - J);
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
    float b = - (1.0 + M_scaled + 100. * a);
    float c = 100. * M_scaled + J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b - root);
  }
}
float aces_find_gamut_boundary_intersection0(vec2 JM_cusp, float gamma_top_inv, float gamma_bottom_inv, float J_intersect_source, float J_intersect_cusp, float slope)
{
  float M_boundary_lower = J_intersect_cusp * pow(J_intersect_source / J_intersect_cusp, gamma_bottom_inv) / (JM_cusp.r / JM_cusp.g - slope);
  float M_boundary_upper = JM_cusp.g * (100. - J_intersect_cusp) * pow((100. - J_intersect_source) / (100. - J_intersect_cusp), gamma_top_inv) / (slope * JM_cusp.g + 100. - JM_cusp.r);
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
float aces_remap_M_inv0(float M, float gamut_boundary_M, float reach_boundary_M)
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
  if (nd >= 1.0f)
  {
    return threshold + scale;
  }
  else
  {
    return threshold + scale * -(nd / (nd - 1.0f));
  }
}
vec3 aces_gamut_compress0(vec3 JMh, float Jx, vec3 JMGcusp, float reachMaxM)
{
  float J = JMh.r;
  float M = JMh.g;
  float h = JMh.b;
  if (M <= 0.0 || J > 100.)
  {
    return vec3(J, 0.0, h);
  }
  else
  {
    vec2 JMcusp = JMGcusp.rg;
    float focusJ = mix(JMcusp.r, 34.096539, min(1.0, 1.300000 - (JMcusp.r / 100.000000)));
    float slope_gain = 135. * aces_get_focus_gain0(Jx, JMcusp.r);
    float J_intersect_source = aces_solve_J_intersect0(JMh.r, JMh.g, focusJ, slope_gain);
    float gamut_slope = (J_intersect_source < focusJ) ? J_intersect_source : (100. - J_intersect_source);
    gamut_slope = gamut_slope * (J_intersect_source - focusJ) / (focusJ * slope_gain);
    float gamma_top_inv = JMGcusp.b;
    float gamma_bottom_inv = 0.877192974;
    float J_intersect_cusp = aces_solve_J_intersect0(JMcusp.r, JMcusp.g, focusJ, slope_gain);
    float gamutBoundaryM = aces_find_gamut_boundary_intersection0(JMcusp, gamma_top_inv, gamma_bottom_inv, J_intersect_source, J_intersect_cusp, gamut_slope);
    if (gamutBoundaryM <= 0.0)
    {
      return vec3(J, 0.0, h);
    }
    float reachBoundaryM = 100. * pow(J_intersect_source / 100.,  0.879464149);
    reachBoundaryM = reachBoundaryM / ((100. / reachMaxM) - gamut_slope);
    float remapped_M = aces_remap_M_inv0(M, gamutBoundaryM, reachBoundaryM);
    float remapped_J = J_intersect_source + remapped_M * gamut_slope;
    return vec3(remapped_J, remapped_M, h);
  }
}
float aces_toe_inv0(float x, float limit, float k1_in, float k2_in)
{
  float k2 = max(k2_in, 0.001);
  float k1 = sqrt(k1_in * k1_in + k2 * k2);
  float k3 = (limit + k1) / (limit + k2);
  return (x > limit) ? x : (x * x + k1 * x) / (k3 * (x + k2));
}

// Declaration of the OCIO shader function

vec4 look_reference(vec4 inPixel)
{
  vec4 outColor = inPixel;
  
  // Add FixedFunction 'ACES_OutputTransform20 (Inverse)' processing
  
  {
    
    // Add RGB to JMh
    
    vec3 JMh;
    vec3 Aab;
    {
      {
        vec3 lms = mat3(0.286458343, 0.110622227, 0.00810814276, 0.385410756, 0.536243081, 0.0489520207, 0.121831447, 0.146835387, 0.736640394) * outColor.rgb;
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
    float reachMaxM = aces_reach_m_table_0_sample(outColor.b);
    
    // Add GamutCompress (inv)
    
    {
      vec3 JMGcusp = aces_gamut_cusp_table_0_sample(outColor.b);
      float Jx = outColor.r;
      vec3 unCompressedJMh;
      if (Jx <= mix(JMGcusp.r, 100.000000, 0.300000))
      {
        unCompressedJMh = aces_gamut_compress0(outColor.rgb, Jx, JMGcusp, reachMaxM);
      }
      else
      {
        Jx = aces_gamut_compress0(outColor.rgb, Jx, JMGcusp, reachMaxM).r;
        unCompressedJMh = aces_gamut_compress0(outColor.rgb, Jx, JMGcusp, reachMaxM);
      }
      outColor.rgb = unCompressedJMh;
    }
    
    // Add ToneScale and ChromaCompress (inv)
    
    float J = aces_tonescale_inv0(outColor.r);
    {
      float J_ts = outColor.r;
      float M_cp = outColor.g;
      float h = outColor.b;
      float M = M_cp;
      if (M_cp != 0.0)
      {
        float nJ = J_ts / 100.;
        float snJ = max(0.0, 1.0 - nJ);
        float Mnorm;
        {
          float cos_hr2 = 2.0 * cos_hr * cos_hr - 1.0;
          float sin_hr2 = 2.0 * cos_hr * sin_hr;
          float cos_hr3 = 4.0 * cos_hr * cos_hr * cos_hr - 3.0 * cos_hr;
          float sin_hr3 = 3.0 * sin_hr - 4.0 * sin_hr * sin_hr * sin_hr;
          vec3 cosines = vec3(cos_hr, cos_hr2, cos_hr3);
          vec3 cosine_weights = vec3(11.341321604032515, 16.469863649185896, 7.8842182208776475);
          vec3 sines = vec3(sin_hr, sin_hr2, sin_hr3);
          vec3 sine_weights = vec3(14.665187919584513, -6.3725780354404442, 9.1941277054452897);
          Mnorm = dot(cosines, cosine_weights) + dot(sines, sine_weights) + 77.133051547393805;
        }
        float limit = pow(nJ, 0.879464149) * reachMaxM / Mnorm;
        M = M_cp / Mnorm;
        M = aces_toe_inv0(M, limit, nJ * 2.4000001, snJ);
        M = limit - aces_toe_inv0(limit - M, limit - 0.001, snJ * 1.29999995, sqrt(nJ * nJ + 0.00499999989));
        M = M * Mnorm;
        M = M * pow(J_ts / J, -0.879464149);
      }
      outColor.rgb = vec3(J, M, h);
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
        JMh.rgb = mat3(2.66705441, -0.535811961, -0.0214488991, -1.52505875, 1.94158089, -0.0485954434, 0.11792551, -0.14584814, 1.32996523) * lms;
      }
      outColor.rgb = JMh;
    }
  }
  
  // Add Matrix processing
  
  {
    vec4 res = vec4(outColor.rgb.r, outColor.rgb.g, outColor.rgb.b, outColor.a);
    vec4 tmp = res;
    res = mat4(1.4514393161456651, -0.076553773396020555, 0.008316148425697719, 0., -0.23651074689374013, 1.1762296998335728, -0.0060324497910210269, 0., -0.2149285692519253, -0.099675926437552215, 0.99771630136532341, 0., 0., 0., 0., 1.) * tmp;
    outColor.rgb = vec3(res.x, res.y, res.z);
    outColor.a = res.w;
  }

  return outColor;
}
